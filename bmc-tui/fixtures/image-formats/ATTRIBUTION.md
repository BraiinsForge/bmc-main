# Image fixture attribution

Every fixture in this directory derives from a single public-domain source, so the provenance below covers all of them.
Derived encodings are re-encodings and rescalings of that source; a derivative of a public-domain work is itself
unencumbered, and the credit is reproduced here regardless.

## Source

**Blue Marble 2002**

- File: <https://commons.wikimedia.org/wiki/File:Blue_Marble_2002.png>
- Direct source used: the 1280px rendition,
  <https://upload.wikimedia.org/wikipedia/commons/thumb/2/23/Blue_Marble_2002.png/1280px-Blue_Marble_2002.png>
- Licence: **Public domain** — a work of the U.S. federal government (NASA)
- Retrieved: 2026-07-24
- Credit: NASA Goddard Space Flight Center. Image by Reto Stöckli (land surface, shallow water, clouds); enhancements by
  Robert Simmon (ocean colour, compositing, 3D globes, animation); data and technical support from the MODIS Land,
  Atmosphere and Ocean groups and the MODIS Science Data Support Team; additional data from the USGS EROS Data Center
  (topography), USGS Terrestrial Remote Sensing Flagstaff Field Center (Antarctica) and the Defense Meteorological
  Satellite Program (city lights). Composited by Wikimedia user Meow.

`blue-marble.png` is that rendition rescaled to 640x320; everything else here is generated from it. The rescale is
deliberate — the corpus needs a real photograph, not a large one, and every stored fixture derives from this file, so
its pixel count sets the whole directory's size.

## Why one source

The corpus needs a file per enabled decoder, and several of those formats — qoi, farbfeld, hdr — have no practical
public sample to fetch. Encoding them all from one image keeps a single, checkable licence story instead of eleven, and
keeps the fixtures visually identical so a format difference is the only variable under test.
