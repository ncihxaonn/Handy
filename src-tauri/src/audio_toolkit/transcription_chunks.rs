use super::{constants::WHISPER_SAMPLE_RATE, VoiceActivityDetector};
use anyhow::{bail, ensure, Result};
use std::ops::Range;

const SAMPLE_RATE: usize = WHISPER_SAMPLE_RATE as usize;
const VAD_FRAME_SAMPLES: usize = SAMPLE_RATE * 30 / 1000;
const MIN_SILENCE_SAMPLES: usize = VAD_FRAME_SAMPLES * 7;

pub fn audio_limit_samples(max_audio_ms: i64) -> Option<usize> {
    (max_audio_ms > 0).then(|| {
        (max_audio_ms as u64)
            .saturating_mul((SAMPLE_RATE / 1000) as u64)
            .min(usize::MAX as u64) as usize
    })
}

pub fn needs_chunking(audio_samples: usize, max_audio_ms: i64) -> bool {
    audio_limit_samples(max_audio_ms).is_some_and(|limit| audio_samples > limit)
}

pub fn ensure_not_cancelled(is_cancelled: &impl Fn() -> bool) -> Result<()> {
    ensure!(!is_cancelled(), "Transcription cancelled");
    Ok(())
}

pub fn validate_audio_length(audio_samples: usize, max_audio_ms: i64) -> Result<()> {
    if let Some(limit) = audio_limit_samples(max_audio_ms) {
        ensure!(
            audio_samples <= limit,
            "Refusing engine call with {} samples: model limit is {} ms ({} samples)",
            audio_samples,
            max_audio_ms,
            limit
        );
    }
    Ok(())
}

pub fn chunk_sample_limit(max_audio_ms: i64) -> Result<usize> {
    let limit = audio_limit_samples(max_audio_ms)
        .ok_or_else(|| anyhow::anyhow!("Cannot chunk a model without an audio limit"))?
        .min(SAMPLE_RATE * 30);
    Ok(limit
        .saturating_sub((limit / 20).min(SAMPLE_RATE / 10))
        .max(1))
}

pub fn plan_vad_chunks(
    audio: &[f32],
    max_samples: usize,
    vad: &mut dyn VoiceActivityDetector,
    is_cancelled: &impl Fn() -> bool,
) -> Result<Vec<Range<usize>>> {
    let mut silences = Vec::new();
    let mut silence_start = None;
    vad.reset();
    for (index, frame) in audio.chunks(VAD_FRAME_SAMPLES).enumerate() {
        ensure_not_cancelled(is_cancelled)?;
        let offset = index * VAD_FRAME_SAMPLES;
        let speech = if frame.len() == VAD_FRAME_SAMPLES {
            vad.is_voice(frame)?
        } else {
            let mut padded = [0.0; VAD_FRAME_SAMPLES];
            padded[..frame.len()].copy_from_slice(frame);
            vad.is_voice(&padded)?
        };
        if speech {
            if let Some(start) = silence_start.take() {
                if offset - start >= MIN_SILENCE_SAMPLES {
                    silences.push(start..offset);
                }
            }
        } else {
            silence_start.get_or_insert(offset);
        }
    }
    if let Some(start) = silence_start {
        if audio.len() - start >= MIN_SILENCE_SAMPLES {
            silences.push(start..audio.len());
        }
    }
    ensure_not_cancelled(is_cancelled)?;
    chunks_at_silences(audio.len(), max_samples, &silences)
}

pub fn chunks_at_silences(
    audio_samples: usize,
    max_samples: usize,
    silences: &[Range<usize>],
) -> Result<Vec<Range<usize>>> {
    ensure!(max_samples > 0, "Audio chunk limit must be positive");
    let mut chunks = Vec::new();
    let mut start = 0;
    while start < audio_samples {
        let hard_end = start.saturating_add(max_samples).min(audio_samples);
        let end = if hard_end == audio_samples {
            hard_end
        } else {
            let earliest_cut = start + max_samples / 2;
            silences
                .iter()
                .rev()
                .find_map(|silence| {
                    let left = silence.start.max(earliest_cut);
                    let right = silence.end.min(hard_end);
                    (right.saturating_sub(left) >= MIN_SILENCE_SAMPLES)
                        .then(|| left + (right - left) / 2)
                })
                .unwrap_or(hard_end)
        };
        chunks.push(start..end);
        start = end;
    }
    Ok(chunks)
}

pub fn transcribe_chunks(
    audio: &[f32],
    chunks: &[Range<usize>],
    max_audio_ms: i64,
    is_cancelled: &impl Fn() -> bool,
    mut transcribe: impl FnMut(&[f32]) -> Result<String>,
    mut progress: impl FnMut(usize, usize),
) -> Result<String> {
    ensure_not_cancelled(is_cancelled)?;
    progress(0, chunks.len());
    let mut text = String::new();
    let mut cursor = 0;
    for (index, chunk) in chunks.iter().enumerate() {
        ensure_not_cancelled(is_cancelled)?;
        ensure!(
            chunk.start == cursor,
            "Audio chunks must be contiguous and ordered"
        );
        ensure!(chunk.end > chunk.start, "Audio chunks must not be empty");
        let samples = audio
            .get(chunk.clone())
            .ok_or_else(|| anyhow::anyhow!("Audio chunk is out of bounds"))?;
        validate_audio_length(samples.len(), max_audio_ms)?;
        let segment_text = transcribe(samples)?;
        ensure_not_cancelled(is_cancelled)?;
        if chunks.len() == 1 {
            text = segment_text;
        } else {
            append_transcription(&mut text, &segment_text);
        }
        cursor = chunk.end;
        progress(index + 1, chunks.len());
    }
    if cursor != audio.len() {
        bail!("Audio chunks did not cover the full recording");
    }
    Ok(text)
}

fn is_cjk(character: char) -> bool {
    matches!(character as u32,
        0x1100..=0x11ff | 0x2e80..=0x2fff | 0x3000..=0x30ff |
        0x3100..=0x31ff | 0x3400..=0x4dbf | 0x4e00..=0x9fff |
        0xa960..=0xa97f | 0xac00..=0xd7ff | 0xf900..=0xfaff |
        0xff00..=0xffef | 0x20000..=0x323af)
}

fn append_transcription(text: &mut String, segment: &str) {
    let segment = segment.trim();
    if segment.is_empty() {
        return;
    }
    if let (Some(left), Some(right)) = (text.chars().last(), segment.chars().next()) {
        if !left.is_whitespace()
            && !is_cjk(left)
            && !is_cjk(right)
            && !",.!?;:)]}".contains(right)
            && !"([{".contains(left)
        {
            text.push(' ');
        }
    }
    text.push_str(segment);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio_toolkit::vad::VadFrame;
    use std::cell::Cell;

    struct TestVad {
        frames: usize,
        cancel_after: Option<usize>,
    }

    impl VoiceActivityDetector for TestVad {
        fn push_frame<'a>(&'a mut self, frame: &'a [f32]) -> Result<VadFrame<'a>> {
            self.frames += 1;
            if self.cancel_after == Some(self.frames) {
                bail!("VAD failed");
            }
            Ok(if frame[0] > 0.0 {
                VadFrame::Speech(frame)
            } else {
                VadFrame::Noise
            })
        }
    }

    #[test]
    fn only_audio_over_a_positive_capability_is_chunked() {
        for limit in [0, -1] {
            assert!(!needs_chunking(SAMPLE_RATE * 1800, limit));
        }
        assert!(!needs_chunking(SAMPLE_RATE * 30, 30_000));
        assert!(needs_chunking(SAMPLE_RATE * 30 + 1, 30_000));
        assert_eq!(chunk_sample_limit(30_000).unwrap(), SAMPLE_RATE * 299 / 10);
        assert!(chunk_sample_limit(10).unwrap() < 160);
    }

    #[test]
    fn thirty_minute_hard_cuts_cover_every_sample_once_under_the_limit() {
        let len = SAMPLE_RATE * 1800 + 37;
        let limit = chunk_sample_limit(30_000).unwrap();
        let chunks = chunks_at_silences(len, limit, &[]).unwrap();
        assert_eq!(chunks.first().unwrap().start, 0);
        assert_eq!(chunks.last().unwrap().end, len);
        assert!(chunks.iter().all(|chunk| chunk.len() <= limit));
        assert!(chunks.windows(2).all(|pair| pair[0].end == pair[1].start));
        assert_eq!(chunks.iter().map(|chunk| chunk.len()).sum::<usize>(), len);
    }

    #[test]
    fn vad_cuts_inside_late_silence_without_dropping_audio_or_tail_samples() {
        let mut audio = vec![1.0; SAMPLE_RATE * 62 + 137];
        audio[SAMPLE_RATE * 26..SAMPLE_RATE * 27].fill(0.0);
        let mut vad = TestVad {
            frames: 0,
            cancel_after: None,
        };
        let limit = chunk_sample_limit(30_000).unwrap();
        let chunks = plan_vad_chunks(&audio, limit, &mut vad, &|| false).unwrap();
        assert!((SAMPLE_RATE * 26..SAMPLE_RATE * 27).contains(&chunks[0].end));
        assert_eq!(chunks.last().unwrap().end, audio.len());
        assert_eq!(vad.frames, audio.len().div_ceil(VAD_FRAME_SAMPLES));
        assert!(chunks.iter().all(|chunk| chunk.len() <= limit));
        assert!(chunks.windows(2).all(|pair| pair[0].end == pair[1].start));
    }

    #[test]
    fn guard_rejects_oversized_calls_before_entering_the_engine() {
        let audio = vec![0.0; SAMPLE_RATE * 31];
        let called = Cell::new(false);
        let result = transcribe_chunks(
            &audio,
            &[0..audio.len()],
            30_000,
            &|| false,
            |_| {
                called.set(true);
                Ok(String::new())
            },
            |_, _| {},
        );
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Refusing engine call"));
        assert!(!called.get());
    }

    #[test]
    fn chunks_run_in_order_and_cancel_stops_remaining_calls_and_progress() {
        let audio = [1.0, 2.0, 3.0];
        let calls = Cell::new(0);
        let cancelled = Cell::new(false);
        let mut progress = Vec::new();
        let result = transcribe_chunks(
            &audio,
            &[0..1, 1..2, 2..3],
            1,
            &|| cancelled.get(),
            |samples| {
                assert_eq!(samples[0], (calls.get() + 1) as f32);
                calls.set(calls.get() + 1);
                cancelled.set(true);
                Ok("partial".into())
            },
            |completed, total| progress.push((completed, total)),
        );
        assert!(result.unwrap_err().to_string().contains("cancelled"));
        assert_eq!(calls.get(), 1);
        assert_eq!(progress, [(0, 3)]);
    }

    #[test]
    fn cancellation_before_vad_does_not_process_frames() {
        let mut vad = TestVad {
            frames: 0,
            cancel_after: None,
        };
        assert!(plan_vad_chunks(&[0.0; VAD_FRAME_SAMPLES], 480, &mut vad, &|| true).is_err());
        assert_eq!(vad.frames, 0);
    }

    #[test]
    fn a_single_short_chunk_keeps_the_original_text_unchanged() {
        let result = transcribe_chunks(
            &[0.0],
            &[0..1],
            30_000,
            &|| false,
            |_| Ok("  Original text.\n".into()),
            |_, _| {},
        )
        .unwrap();
        assert_eq!(result, "  Original text.\n");
    }

    #[test]
    fn successful_chunks_preserve_order_and_report_completion() {
        let mut progress = Vec::new();
        let text = transcribe_chunks(
            &[1.0, 2.0, 3.0],
            &[0..1, 1..2, 2..3],
            1,
            &|| false,
            |samples| Ok(format!("segment{}", samples[0])),
            |completed, total| progress.push((completed, total)),
        )
        .unwrap();
        assert_eq!(text, "segment1 segment2 segment3");
        assert_eq!(progress, [(0, 3), (1, 3), (2, 3), (3, 3)]);
    }

    #[test]
    #[ignore = "requires HANDY_TEST_MODEL_PATH and HANDY_TEST_AUDIO_PATH"]
    fn sensevoice_metal_cancel_stops_after_one_real_segment() {
        use transcribe_cpp::{CancelToken, Model, RunOptions};

        let model = Model::load(std::env::var("HANDY_TEST_MODEL_PATH").unwrap()).unwrap();
        assert_eq!(model.arch(), "sensevoice");
        assert!(model.backend().starts_with("MTL"));
        let max_audio_ms = model.capabilities().max_audio_ms;
        let audio = super::super::read_wav_samples(std::path::Path::new(
            &std::env::var("HANDY_TEST_AUDIO_PATH").unwrap(),
        ))
        .unwrap();
        let mut vad = super::super::SileroVad::new(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("resources/models/silero_vad_v4.onnx"),
            0.5,
        )
        .unwrap();
        let cancel = CancelToken::new();
        let chunks = plan_vad_chunks(
            &audio,
            chunk_sample_limit(max_audio_ms).unwrap(),
            &mut vad,
            &|| false,
        )
        .unwrap();
        assert!(chunks.len() > 1);
        let mut session = model.session().unwrap();
        session.set_cancel_token(&cancel);
        let mut calls = 0;
        let result = transcribe_chunks(
            &audio,
            &chunks,
            max_audio_ms,
            &|| cancel.is_cancelled(),
            |samples| {
                let result = session.run(samples, &RunOptions::default())?;
                assert!(!result.text.trim().is_empty());
                calls += 1;
                cancel.cancel();
                Ok(result.text)
            },
            |_, _| {},
        );
        assert!(result.unwrap_err().to_string().contains("cancelled"));
        assert_eq!(calls, 1);
        session.clear_cancel_token();
        assert!(!session
            .run(&audio[..SAMPLE_RATE * 15], &RunOptions::default())
            .unwrap()
            .text
            .trim()
            .is_empty());
        eprintln!(
            "Real Metal cancellation: 1/{} segments ran; session reuse passed",
            chunks.len()
        );
    }

    #[test]
    fn sequential_text_joining_handles_latin_cjk_and_empty_segments() {
        for (parts, expected) in [
            (vec![" Hello. ", "", "World!"], "Hello. World!"),
            (vec!["你好，", "世界。", "下一句。"], "你好，世界。下一句。"),
            (vec!["日本語", "です。"], "日本語です。"),
            (vec!["한국어", "입니다."], "한국어입니다."),
            (vec!["Hello", ", world."], "Hello, world."),
        ] {
            let mut text = String::new();
            for part in parts {
                append_transcription(&mut text, part);
            }
            assert_eq!(text, expected);
        }
    }

    #[test]
    fn missing_or_out_of_order_audio_is_not_silently_accepted() {
        for chunks in [vec![1..2], vec![0..1], vec![0..0], vec![0..3]] {
            assert!(transcribe_chunks(
                &[0.0, 0.0],
                &chunks,
                0,
                &|| false,
                |_| Ok("text".into()),
                |_, _| {}
            )
            .is_err());
        }
    }
}
