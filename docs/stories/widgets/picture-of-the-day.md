# Picture of the Day Widget

The Picture of the Day widget shows the current NASA Astronomy Picture of the Day, fitted to the widget, captioned with
its title and, where the feed names one, the photographer's credit. It follows the feed rather than the clock: the Deck
asks a metadata endpoint every half hour what the current picture is, and downloads a picture only when that answer
names a date it is not already showing.

## User stories

### See today's astronomy picture

> As a user, I want the Deck to show the current NASA picture of the day, so my Deck carries something new to look at
> each day without me touching it.

- The picture is served by the Braiins Forge Nexus, already re-encoded to the widget's pixel size, so the Deck never
  downloads a full-resolution original.
- A new picture appears within half an hour of being published. No configuration is involved: adding the widget is the
  whole setup.
- While the first picture is still loading, the widget shows a placeholder rather than a blank panel.

### Know what I am looking at, and who took it

> As a user, I want the picture's title and its credit on screen, so I know what the image is and whose work it is.

- The *title* is drawn in the top-left corner in white, and can be turned off with a *Show title* option.
- The *credit* is drawn in the bottom-right corner, smaller and in grey, on a dark translucent plate so it stays
  readable over a bright picture. It **cannot** be hidden — many of these pictures are copyrighted by the photographer
  rather than being public domain. An entry the feed publishes without a copyright line, a public-domain NASA image,
  shows no credit at all.
- Both are drawn at a fixed type size and a fixed inset from the edge, chosen to stay readable on the narrowest tile a
  widget can occupy. They do not grow with the viewport, so on the full-screen face the caption reads as a small label
  in the corner rather than a headline.
- A caption is only ever drawn against the picture it describes. If the two ever disagree, the picture is shown without
  a caption.

### Keep the picture through a restart and an outage

> As a user, I want the picture on screen to survive a reboot and a network outage, so the widget carries a photograph
> rather than a status message whenever the Deck cannot reach the feed.

- The picture is cached on flash and shown immediately on restart or when the widget returns from being off-screen — no
  re-fetch and no waiting on the network on that path.
- A check that names the date already on screen changes nothing. Only a corrected title or credit is taken from it.
- While the feed cannot be reached the picture stays up untouched, with nothing drawn over it, and the checks resume on
  their own every thirty seconds. So a Deck that started before its network reaches the first picture shortly after the
  network comes up, rather than waiting out a full check.
- When the date does move, the widget clears to *Loading image* for the few seconds the new picture takes to download
  and decode, then shows it. Holding the old picture under an *Updating* overlay across that gap is not implemented yet.
  A download that fails leaves *Failed to load image* on screen and retries — after five minutes if it reached nothing,
  at the next feed check if what arrived was a broken picture.
- Tapping the picture reveals a menu with *Reload*, which re-checks the feed and re-downloads immediately. The current
  picture stays up under an *Updating* overlay while that runs, and a reload that cannot reach the feed leaves it in
  place, tagged *Last refresh N ago* once the picture on screen is more than a day and a half old.

## Constraints

- The widget never computes a date. The feed states the published date and the Deck only compares it against the date of
  the picture it already holds, so an unsynchronised device clock cannot make it ask for the wrong day.
- Because the picture URL carries that date, a publication landing between the two requests cannot file a new picture
  under the old date.
- One picture is downloaded per publication, not per check: the half-hourly check costs about a kilobyte, and the
  picture itself is fetched once a day at most. The decode and the flash write happen only on that fetch.
- Pictures are requested as JPEG at the widget's pixel size, and arrive scaled to fit inside it with the aspect ratio
  preserved. There is no fill-and-crop option: the picture is shown whole, letterboxed where its shape differs from the
  widget's. Asking for JPEG also matters — the host rejects sources over its decode budget, and JPEG is the format that
  budget is most generous with.
- *Source* is a single-value option today (NASA Astronomy Picture of the Day). It exists so further feeds can be added
  without a second widget.
- The cached picture is one viewport-sized artifact per widget instance, stored alongside a small caption record under
  `/mnt/data/bmc/widget-cache/` and reclaimed by the host's asset-cache GC when the widget is removed.
- Rectangular viewports only.
