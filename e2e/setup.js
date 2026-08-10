const fs = require("fs");
const path = require("path");
const { execSync } = require("child_process");

const SHARE_DIR = process.env.WP_SHARE_DIR || "/tmp/hostshare";

function exists(p) {
  try {
    return fs.statSync(p).size > 0;
  } catch {
    return false;
  }
}

function ffmpeg(args) {
  execSync("ffmpeg -y " + args, { stdio: "inherit" });
}

function clean() {
  const keep = ["clip.mp4", "mov.avi", "pic.png"];
  for (const f of fs.readdirSync(SHARE_DIR)) {
    if (!keep.includes(f)) {
      fs.rmSync(path.join(SHARE_DIR, f), { recursive: true, force: true });
    }
  }
}

function fixtures() {
  fs.mkdirSync(SHARE_DIR, { recursive: true });
  if (!exists(path.join(SHARE_DIR, "clip.mp4"))) {
    console.log("[setup] generating clip.mp4");
    ffmpeg(`-f lavfi -i testsrc2=duration=1:size=320x240:rate=24 -pix_fmt yuv420p "${SHARE_DIR}/clip.mp4"`);
  }
  if (!exists(path.join(SHARE_DIR, "mov.avi"))) {
    console.log("[setup] generating mov.avi");
    ffmpeg(`-f lavfi -i testsrc2=duration=2:size=320x240:rate=24 -c:v mpeg4 "${SHARE_DIR}/mov.avi"`);
  }
  if (!exists(path.join(SHARE_DIR, "pic.png"))) {
    console.log("[setup] generating pic.png");
    ffmpeg(`-f lavfi -i testsrc2=duration=0.1:size=64x64 -frames:v 1 "${SHARE_DIR}/pic.png"`);
  }
}

clean();
fixtures();
console.log("[setup] fixtures ready in " + SHARE_DIR);
