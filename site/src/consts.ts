export const SITE_TITLE = "Openstreamer";

// English fallback description. Per-locale copy lives in src/i18n/locales/*.json
// (site.description) and is what the pages actually render.
export const SITE_DESCRIPTION =
  "Raspberry Pi 4 software that takes the live feed from DJI FPV goggles or a USB camera and sends it to HDMI, any browser, RTSP/SRT/RTMP/UDP streams and recordings.";

export const INSTALL_COMMAND = "sudo apt install ./openstreamer_*.deb";

export type TestedResult =
  "worksFull" | "worksLimited" | "worksPartial" | "doesntWork";
export type TestedBy = "maintainer" | "community";

export interface TestedSetup {
  device: string;
  airUnits?: string[];
  piRevision?: string;
  result: TestedResult;
  by: TestedBy;
  notes?: string;
}

export const TESTED_HARDWARE: TestedSetup[] = [
  {
    device: "DJI Goggles Integra",
    airUnits: ["O3 Air Unit", "O4 Lite", "O4  Pro"],
    piRevision: "1.5",
    result: "worksFull",
    by: "maintainer",
  },
  {
    device: "Eachine ROTG02 Analog Receiver",
    piRevision: "1.5",
    result: "worksFull",
    by: "maintainer",
  },
  {
    device: "DJI Action 4 (Webcam mode)",
    piRevision: "1.5",
    result: "worksLimited",
    by: "maintainer",
  },
];
