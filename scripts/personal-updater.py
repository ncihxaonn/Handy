import base64
import ctypes
import hmac
import json
import os
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
REPO = "ncihxaonn/Handy"
SERVICE = b"personal.ncihxaonn.Handy.updater"
ACCOUNT = REPO.encode()
HELPER = Path.home() / "Documents/Codex/bin/codex-project-credential"
GH = ["env", f"GH_CONFIG_DIR={Path.home()}/.github-profiles/personal", "gh"]


def scope(remote=False):
    profile = subprocess.check_output([str(HELPER), "profile"], cwd=ROOT, text=True).strip()
    origin = subprocess.check_output(["git", "remote", "get-url", "origin"], cwd=ROOT, text=True).strip()
    if profile != "personal" or origin != f"https://github.com/{REPO}.git":
        raise RuntimeError("Personal Handy credential scope mismatch")
    if remote:
        login = subprocess.check_output(GH + ["api", "user", "--jq", ".login"], text=True).strip()
        repo = subprocess.check_output(GH + ["repo", "view", REPO, "--json", "nameWithOwner", "--jq", ".nameWithOwner"], text=True).strip()
        if login != "ncihxaonn" or repo != REPO:
            raise RuntimeError("Personal GitHub account or repository mismatch")


def keychain():
    api = ctypes.CDLL("/System/Library/Frameworks/Security.framework/Security")
    api.SecKeychainFindGenericPassword.argtypes = [ctypes.c_void_p, ctypes.c_uint32, ctypes.c_char_p, ctypes.c_uint32, ctypes.c_char_p, ctypes.POINTER(ctypes.c_uint32), ctypes.POINTER(ctypes.c_void_p), ctypes.c_void_p]
    api.SecKeychainFindGenericPassword.restype = ctypes.c_int32
    api.SecKeychainAddGenericPassword.argtypes = [ctypes.c_void_p, ctypes.c_uint32, ctypes.c_char_p, ctypes.c_uint32, ctypes.c_char_p, ctypes.c_uint32, ctypes.c_char_p, ctypes.c_void_p]
    api.SecKeychainAddGenericPassword.restype = ctypes.c_int32
    api.SecKeychainItemFreeContent.argtypes = [ctypes.c_void_p, ctypes.c_void_p]
    api.SecKeychainItemFreeContent.restype = ctypes.c_int32
    return api


def read_key(api):
    length = ctypes.c_uint32()
    data = ctypes.c_void_p()
    status = api.SecKeychainFindGenericPassword(None, len(SERVICE), SERVICE, len(ACCOUNT), ACCOUNT, ctypes.byref(length), ctypes.byref(data), None)
    if status == -25300:
        return None
    if status != 0:
        raise RuntimeError(f"Keychain read failed: OSStatus {status}")
    try:
        return ctypes.string_at(data, length.value)
    finally:
        api.SecKeychainItemFreeContent(None, data)


def generate_key():
    result = subprocess.run(["bun", "run", "tauri", "signer", "generate", "--ci", "--password", ""], cwd=ROOT, capture_output=True, text=True)
    if result.returncode != 0:
        raise RuntimeError("Tauri updater key generation failed")
    keys = {}
    for candidate in re.findall(r"[A-Za-z0-9+/=]{100,}", result.stdout):
        try:
            header = base64.b64decode(candidate, validate=True).split(b"\n", 1)[0]
        except ValueError:
            continue
        if b"secret key" in header:
            keys["private_key"] = candidate
        elif b"public key" in header:
            keys["public_key"] = candidate
    if set(keys) != {"private_key", "public_key"}:
        raise RuntimeError("Could not parse generated updater key pair; no key material was logged")
    return json.dumps(keys, sort_keys=True).encode()


def main():
    if len(sys.argv) != 2 or sys.argv[1] not in {"setup", "check", "build"}:
        raise RuntimeError("Usage: python3 scripts/personal-updater.py setup|check|build")
    action = sys.argv[1]
    scope(remote=action == "setup")
    api = keychain()
    saved = read_key(api)
    if action == "setup" and saved is None:
        generated = generate_key()
        status = api.SecKeychainAddGenericPassword(None, len(SERVICE), SERVICE, len(ACCOUNT), ACCOUNT, len(generated), generated, None)
        if status != 0:
            raise RuntimeError(f"Keychain save failed: OSStatus {status}")
        saved = read_key(api)
        if saved is None or not hmac.compare_digest(saved, generated):
            raise RuntimeError("Keychain save could not be verified")
    if saved is None:
        raise RuntimeError("Dedicated Personal Handy updater key is absent from Keychain")
    keys = json.loads(saved)
    if action == "setup":
        scope(remote=True)
        subprocess.run(GH + ["secret", "set", "TAURI_SIGNING_PRIVATE_KEY", "--repo", REPO], input=keys["private_key"].encode(), check=True)
        secrets = json.loads(subprocess.check_output(GH + ["secret", "list", "--repo", REPO, "--json", "name"], text=True))
        if not any(secret["name"] == "TAURI_SIGNING_PRIVATE_KEY" for secret in secrets):
            raise RuntimeError("GitHub updater secret metadata could not be verified")
        print(json.dumps({"status": "saved", "keychain_service": SERVICE.decode(), "github_secret": "TAURI_SIGNING_PRIVATE_KEY", "public_key": keys["public_key"]}))
        return
    config = json.loads((ROOT / "src-tauri/tauri.conf.json").read_text())
    if config["plugins"]["updater"]["pubkey"] != keys["public_key"]:
        raise RuntimeError("Configured updater public key does not match the dedicated Keychain item")
    if action == "check":
        print(json.dumps({"profile": "personal", "repository": REPO, "keychain": "present", "public_key_matches": True}))
        return
    environment = os.environ.copy()
    environment["TAURI_SIGNING_PRIVATE_KEY"] = keys["private_key"]
    environment["TAURI_SIGNING_PRIVATE_KEY_PASSWORD"] = ""
    raise SystemExit(subprocess.call(["bun", "run", "tauri", "build", "--bundles", "app", "--ci"], cwd=ROOT, env=environment))


if __name__ == "__main__":
    try:
        main()
    except Exception as error:
        raise SystemExit(str(error)) from None
