import React from "react";
import { useTranslation } from "react-i18next";

interface UpdateCheckerProps {
  className?: string;
}

const UpdateChecker: React.FC<UpdateCheckerProps> = ({ className = "" }) => {
  const { t } = useTranslation();

  return (
    <div className={`flex items-center gap-3 ${className}`}>
      <span className="text-text/60 tabular-nums">
        {t("footer.updateCheckingDisabled")}
      </span>
    </div>
  );
};

export default UpdateChecker;
