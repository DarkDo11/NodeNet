import { hasFlag } from "country-flag-icons";
import * as FlagIcons from "country-flag-icons/react/3x2";

interface CountryFlagProps {
  country: string;
  className?: string;
}

const normalizeCountryCode = (country: string) => {
  const code = country.trim().toUpperCase();
  if (code === "UK") return "GB";
  if (code === "EL") return "GR";
  return code;
};

export default function CountryFlag({ country, className = "" }: CountryFlagProps) {
  const code = normalizeCountryCode(country);
  const classes = ["country-flag-icon", className].filter(Boolean).join(" ");

  if (/^[A-Z]{2}$/.test(code) && hasFlag(code)) {
    const Flag = FlagIcons[code as keyof typeof FlagIcons];
    return (
      <span className={classes} title={code} aria-label={code}>
        <Flag />
      </span>
    );
  }

  return (
    <span className={`${classes} fallback`} title={code || "Unknown country"} aria-label={code || "Unknown country"}>
      {code || "--"}
    </span>
  );
}
