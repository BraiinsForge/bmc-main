---
name: video-frame-extraction
description: Extract frames from a video file the user pointed at, so they can be inspected visually with the file-read tool. Triggers when the user supplies a path to a video file (.webm/.mp4/.mov/.mkv/.avi/.gif) for any kind of visual inspection — bug screencasts, UI captures, recordings attached to a ticket, etc.
---

# Video frame extraction

## When to use

User hands over a video file path and wants the agent to *see* what's in it — most commonly a screencast attached to a
bug report. Triggers: "frames from this video", "what does the recording show", "extract images from <path>.webm", or
simply giving a path to a video after agreeing to inspect it.

If the user wants to *transcode* or *edit* the video (cut, re-encode, change format), this skill doesn't apply — just
use ffmpeg directly.

## Why a project-local output directory

The file-read tool prompts for permission on paths outside the project's working directory. Reading PNGs from `/tmp/...`
or `~/Downloads/...` interrupts the flow on every frame. Writing frames into the **current project directory** (the cwd
the agent was launched in) sidesteps this entirely.

**Output root:** `./.ai/scratch/video-frames/<slug>/`

- `./` = the project root (working directory). Don't substitute `$HOME`, `/tmp`, or the video's own directory.
- `.ai/scratch/` is gitignored in this repo (see `.gitignore`) — the agent's scratch area, safe to write to without
  polluting the repo. `.ai/` is the one real shared-agent directory; `.claude/` and `.agents/` are just symlink shims
  into it, so scratch output does not belong there.
- `<slug>` = a short identifier for this clip (e.g. ticket key `bdk-527`, or `frames-1` if nothing else fits). Keeps
  multiple clips in the same session from clobbering each other.

If `.ai/scratch/` doesn't exist yet, create it (`mkdir -p ./.ai/scratch/video-frames/<slug>`). Do not write outside the
project root.

## Workflow

### 1. Inspect the video first

Don't extract blindly. Read duration, resolution, and (if non-degenerate) frame rate:

```bash
ffprobe -v error \
  -show_entries stream=width,height,r_frame_rate \
  -show_entries format=duration \
  -of default <path>
```

Screencasts often report a bogus `r_frame_rate` like `1000/1` (variable frame rate) — ignore that and rely on
`duration`.

### 2. Go straight to a dense pass, then ask the user where the effect is

Skip coarse-pass triangulation. Most UI bugs are sub-200ms transitions that sit *between* low-fps frames, producing 5–10
near-identical coarse shots and wasted read-rounds. Go dense up front, then ask the user — they lived through the clip
and can point at a frame number in one sentence.

```bash
rm -rf ./.ai/scratch/video-frames/<slug>
mkdir -p ./.ai/scratch/video-frames/<slug>
ffmpeg -loglevel error -i <path> -vf "fps=15" \
  ./.ai/scratch/video-frames/<slug>/d_%03d.png
```

Report back: *"Frames at `./.ai/scratch/video-frames/<slug>/d_NNN.png`, NNN total. Which frame(s) does the effect show
up at?"*

Pick `fps=15` for most clips. For very long clips (>30 s) drop to `fps=10` to keep the frame count manageable; for very
short GIFs (\<2 s) bump to `fps=20`.

### 3. Reading frames

Reading a PNG returns it as an image. Read the frames the user pointed at plus 1–2 neighbours on each side to see the
trajectory. Avoid speculative reads — the conversation balloons fast.

When describing the sequence to the user, build a small table mapping frame → state
(`d_054: hands at 12:30 | d_055: hands swung to 3 o'clock | d_057: hands back to 12:30`). That's worth far more than
dumping every screenshot. Note specifically what *changed* between adjacent frames — "the position of X moved" vs "the
angle of Y rotated" vs "the colour shifted" — since the user often has a more precise model of the failure than the raw
pixels reveal.

### 4. Targeted zoom only if the dense pass isn't enough

If 15 fps still misses a transition (rare for UI bugs but possible for sub-100ms snaps, or for reading small UI details
at the source resolution), re-extract a narrow time window with cropping:

```bash
rm -rf ./.ai/scratch/video-frames/<slug>-zoom
mkdir -p ./.ai/scratch/video-frames/<slug>-zoom
ffmpeg -loglevel error \
  -ss <start_sec> -to <end_sec> \
  -i <path> \
  -vf "fps=<dense_rate>,crop=<w>:<h>:<x>:<y>" \
  ./.ai/scratch/video-frames/<slug>-zoom/zoom_%02d.png
```

- `crop=W:H:X:Y` — `X`/`Y` are top-left in pixels of the source frame. If you guess the offset wrong, the relevant row
  will be cut in half — re-run with adjusted `Y`.
- Bump `fps` to 30 only if frames at 15 fps look identical but you know there's a transition between them.

### 5. Cleanup

Leave the frames in `./.ai/scratch/video-frames/` for the rest of the session — useful if the user asks a follow-up
about a different timestamp. They're gitignored, so they won't be committed; don't `rm -rf` the directory automatically.
If the user wants it gone, they can ask.

## Common pitfalls

- **Coarse-pass triangulation.** Extracting at 1–2 fps then reading first/middle/last to find the bug. Most UI
  transitions are sub-200ms — they sit *between* coarse frames. Go dense up front and ask the user which frame.
- **Extracting to `/tmp/` out of habit.** Every subsequent read then prompts for permission. Always write under
  `./.ai/scratch/video-frames/`.
- **Trusting `r_frame_rate` from ffprobe.** For screencasts this is meaningless. Use `duration` to plan fps.
- **Cropping before knowing where to look.** If the resolution makes the region obvious, crop up front; otherwise do an
  uncropped dense pass first.
- **Extracting one frame at a time** (`-ss <t> -frames:v 1`) in a loop. Slow and fiddly. Use a single ffmpeg invocation
  with `fps=N` and a sprintf pattern; cheaper and reproducible.
- **Reading every frame.** Read the frames the user names plus 1–2 neighbours; describe the rest from filenames if they
  look identical.

## Audio / GIFs / other formats

- `.gif`: same flow, ffmpeg handles it. Often very short, bump to `fps=20` since the whole clip may be < 2 s.
- `.mp4`/`.mov`/`.mkv`/`.webm`: identical workflow.
- Audio tracks: ignored — frames are visual only. If the user explicitly wants audio transcription, that's a different
  task.
