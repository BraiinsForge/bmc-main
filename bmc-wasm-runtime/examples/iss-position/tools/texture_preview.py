#!/usr/bin/env python3

"""Interactive globe + flat viewport viewer for earth texture comparison.

Usage:
    python tools/texture_preview.py [--port PORT]

Generates an HTML viewer and serves it on localhost. Opens in browser.
Run texture_download.py first to fetch the textures.
"""

import argparse
import functools
import http.server
import json
import threading
import webbrowser

from _textures import TARGET_H, TARGET_W, TEXTURE_DIR, TEXTURES

VIEWER_HTML: str = str(TEXTURE_DIR / 'viewer.html')


def generate_viewer() -> None:
    texture_meta: str = json.dumps(
        [
            {
                'id': t['id'],
                'name': t['name'],
                'license': t['license'],
                'note': t['note'],
                'native_res': t['native_res'],
                'projection': t['projection'],
            }
            for t in TEXTURES
        ],
        indent=2,
    )

    # language=html
    html = f"""\
<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<title>Earth Texture Comparison \u2014 Globe + Flat Viewport</title>
<style>
  * {{ margin: 0; padding: 0; box-sizing: border-box; }}
  body {{ background: #111; color: #eee; font-family: system-ui, sans-serif; }}

  #top-bar {{
    position: sticky; top: 0; z-index: 10;
    background: rgba(0,0,0,0.92);
  }}

  #tex-grid {{
    display: flex; gap: 6px; padding: 10px 16px 6px;
    flex-wrap: wrap;
  }}
  .tex-card {{
    cursor: pointer; flex-shrink: 0;
    border: 2px solid #333; border-radius: 6px;
    overflow: hidden; background: #1a1a1a;
    transition: border-color 0.15s, box-shadow 0.15s;
  }}
  .tex-card:hover {{ border-color: #555; }}
  .tex-card:has(input:checked) {{
    border-color: #4488ff;
    box-shadow: 0 0 8px rgba(68, 136, 255, 0.3);
  }}
  .tex-card input[type="radio"] {{ display: none; }}
  .tex-card img {{
    width: 140px; height: 70px; object-fit: cover; display: block;
    background: #0a0a14;
  }}
  .tex-name {{
    font-size: 10px; padding: 3px 4px; color: #999;
    white-space: nowrap; overflow: hidden; text-overflow: ellipsis;
    width: 140px; text-align: center; display: block;
  }}

  #controls {{
    padding: 6px 16px 8px;
    display: flex; gap: 20px; align-items: center; flex-wrap: wrap;
    border-top: 1px solid #222;
  }}
  #controls label {{
    display: flex; align-items: center; gap: 6px;
    font-size: 13px; color: #999; white-space: nowrap;
  }}
  #controls select {{
    font-size: 13px; padding: 3px 6px;
    background: #222; color: #eee; border: 1px solid #444; border-radius: 3px;
  }}
  #controls input[type=range] {{ width: 100px; }}

  #viewport {{
    display: flex; gap: 24px; padding: 24px 24px 48px;
    justify-content: center; align-items: flex-start; flex-wrap: wrap;
  }}
  .panel {{ text-align: center; position: relative; }}
  .panel h3 {{ font-size: 13px; color: #666; margin-bottom: 8px; font-weight: 400; }}
  canvas {{ border: 1px solid #333; border-radius: 4px; display: block; }}

  #globe-overlay {{
    position: absolute; inset: 0; top: 28px;
    display: none; align-items: center; justify-content: center;
    color: #666; font-size: 14px; pointer-events: none;
    background: rgba(17,17,17,0.85); border-radius: 4px;
  }}

  #size-info {{
    position: fixed; bottom: 0; left: 0; right: 0;
    background: rgba(0,0,0,0.9); padding: 8px 16px;
    font-size: 12px; color: #555;
    display: flex; gap: 24px; align-items: center;
  }}
  #size-info .note {{ color: #777; }}
  #size-info .warn {{ color: #a86; }}
</style>
</head>
<body>

<div id="top-bar">
  <div id="tex-grid"></div>
  <div id="controls">
    <label>Lat
      <input type="range" id="flat-lat" min="-60" max="60" value="0" step="0.5">
      <span id="flat-lat-val" style="font-variant-numeric:tabular-nums">0.0\u00b0</span>
    </label>
    <label>Lon
      <input type="range" id="flat-lon" min="-180" max="180" value="0" step="0.5">
      <span id="flat-lon-val" style="font-variant-numeric:tabular-nums">0.0\u00b0</span>
    </label>
    <label><input type="checkbox" id="show-grid"> Grid</label>
    <label><input type="checkbox" id="auto-rotate" checked> Rotate</label>
  </div>
</div>

<div id="viewport">
  <div class="panel">
    <h3>3D Globe (drag to rotate, scroll to zoom)</h3>
    <canvas id="globe-canvas" width="560" height="560"></canvas>
    <div id="globe-overlay">
      Web Mercator projection \u2014 cannot display on equirectangular globe
    </div>
  </div>
  <div class="panel">
    <h3>Flat Viewport (560\u00d7480 \u2014 actual widget size)</h3>
    <canvas id="flat-canvas" width="560" height="480"></canvas>
  </div>
</div>

<div id="size-info">
  <span id="file-size">\u2014</span>
  <span class="note" id="tex-note">\u2014</span>
  <span id="tex-license">\u2014</span>
  <span class="warn" id="proj-warn"></span>
</div>

<script type="importmap">
{{
  "imports": {{
    "three": "https://cdn.jsdelivr.net/npm/three@0.170.0/build/three.module.js",
    "three/addons/": "https://cdn.jsdelivr.net/npm/three@0.170.0/examples/jsm/"
  }}
}}
</script>

<script type="module">
import * as THREE from 'three';
import {{ OrbitControls }} from 'three/addons/controls/OrbitControls.js';

const TEXTURES = {texture_meta};
const TARGET_W = {TARGET_W};
const TARGET_H = {TARGET_H};

// \u2500\u2500 Texture grid (radio cards) \u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500
const texGrid = document.getElementById('tex-grid');
TEXTURES.forEach((t, i) => {{
  const label = document.createElement('label');
  label.className = 'tex-card';
  label.innerHTML = `
    <input type="radio" name="texture" value="${{t.id}}" ${{i === 0 ? 'checked' : ''}}>
    <img src="${{t.id}}.jpg" alt="${{t.name}}" loading="lazy">
    <span class="tex-name">${{t.name}}</span>
  `;
  texGrid.appendChild(label);
}});

// \u2500\u2500 Globe \u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500
const globeCanvas = document.getElementById('globe-canvas');
const renderer = new THREE.WebGLRenderer({{ canvas: globeCanvas, antialias: true }});
renderer.setPixelRatio(window.devicePixelRatio);
renderer.setSize(560, 560);
const maxAniso = renderer.capabilities.getMaxAnisotropy();

const scene = new THREE.Scene();
scene.background = new THREE.Color(0x111111);
const camera = new THREE.PerspectiveCamera(45, 1, 0.1, 100);
camera.position.set(0, 0, 3);

const orbitCtrl = new OrbitControls(camera, globeCanvas);
orbitCtrl.enableDamping = true;
orbitCtrl.dampingFactor = 0.05;
orbitCtrl.minDistance = 1.5;
orbitCtrl.maxDistance = 8;

scene.add(new THREE.AmbientLight(0xffffff, 2.0));

const geometry = new THREE.SphereGeometry(1, 64, 64);
const material = new THREE.MeshStandardMaterial({{ roughness: 1, metalness: 0 }});
const globe = new THREE.Mesh(geometry, material);
scene.add(globe);

// Grid
const gridGroup = new THREE.Group();
globe.add(gridGroup);
(function buildGrid() {{
  const mat = new THREE.LineBasicMaterial({{ color: 0x444444, transparent: true, opacity: 0.4 }});
  for (let lat = -60; lat <= 60; lat += 30) {{
    const r = Math.cos(lat * Math.PI / 180), y = Math.sin(lat * Math.PI / 180);
    const pts = Array.from({{ length: 65 }}, (_, i) => {{
      const a = (i / 64) * Math.PI * 2;
      return new THREE.Vector3(r * Math.cos(a), y, r * Math.sin(a));
    }});
    gridGroup.add(new THREE.Line(new THREE.BufferGeometry().setFromPoints(pts), mat));
  }}
  for (let lon = 0; lon < 360; lon += 30) {{
    const a = lon * Math.PI / 180;
    const pts = Array.from({{ length: 65 }}, (_, i) => {{
      const lat = (i / 64) * Math.PI - Math.PI / 2;
      return new THREE.Vector3(Math.cos(lat) * Math.cos(a), Math.sin(lat), Math.cos(lat) * Math.sin(a));
    }});
    gridGroup.add(new THREE.Line(new THREE.BufferGeometry().setFromPoints(pts), mat));
  }}
}})();
gridGroup.visible = false;

// \u2500\u2500 Flat viewport \u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500
const flatCanvas = document.getElementById('flat-canvas');
const ctx = flatCanvas.getContext('2d');
let flatImg = null;
let currentProj = 'equirectangular';

function drawFlat() {{
  if (!flatImg) return;
  const lat = parseFloat(document.getElementById('flat-lat').value);
  const lon = parseFloat(document.getElementById('flat-lon').value);

  ctx.fillStyle = '#0a0a14';
  ctx.fillRect(0, 0, 560, 480);

  if (currentProj === 'mercator') {{
    // Mercator: square tile, 512 CSS px = full world width at zoom 1.
    const W = 512, H = 512;
    const imgX = 280 - ((lon + 180) / 360) * W;
    const imgY = 240 - H / 2;
    for (let dx = -1; dx <= 1; dx++) {{
      const x = imgX + dx * W;
      if (x + W >= 0 && x <= 560) ctx.drawImage(flatImg, x, imgY, W, H);
    }}
  }} else {{
    // Equirectangular: 360\u00b0\u00d7180\u00b0, uniform scale.
    // At zoom 1: 512 CSS px = 360\u00b0 width, 256 CSS px = 180\u00b0 height.
    const W = 512, H = 256;
    const imgX = 280 - ((lon + 180) / 360) * W;
    const imgY = 240 - ((90 - lat) / 180) * H;
    for (let dx = -1; dx <= 1; dx++) {{
      const x = imgX + dx * W;
      if (x + W >= 0 && x <= 560) ctx.drawImage(flatImg, x, imgY, W, H);
    }}
  }}

  // Crosshair at center
  ctx.strokeStyle = 'rgba(18, 67, 205, 0.8)';
  ctx.lineWidth = 1;
  ctx.beginPath(); ctx.arc(280, 240, 24, 0, Math.PI * 2); ctx.stroke();
  ctx.beginPath();
  ctx.moveTo(248, 240); ctx.lineTo(312, 240);
  ctx.moveTo(280, 208); ctx.lineTo(280, 272);
  ctx.stroke();
}}

// \u2500\u2500 Texture loading \u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500

function getSelectedId() {{
  const checked = document.querySelector('input[name="texture"]:checked');
  return checked ? checked.value : TEXTURES[0].id;
}}

async function loadTexture() {{
  const id = getSelectedId();
  const meta = TEXTURES.find(t => t.id === id);
  currentProj = meta.projection;
  const isMerc = currentProj === 'mercator';
  const src = id + '.jpg';

  document.getElementById('tex-note').textContent = meta.note;
  document.getElementById('tex-license').textContent = meta.license;
  document.getElementById('proj-warn').textContent = isMerc ? '\u26a0 Mercator \u2014 poles distorted on globe' : '';
  document.getElementById('globe-overlay').style.display = 'none';

  // Globe
  globe.visible = true;
  material.color.set(0xffffff);
  try {{
    const tex = await new THREE.TextureLoader().loadAsync(src);
    tex.colorSpace = THREE.SRGBColorSpace;
    tex.anisotropy = maxAniso;
    tex.minFilter = THREE.LinearFilter;
    tex.magFilter = THREE.LinearFilter;
    tex.generateMipmaps = false;
    material.map = tex;
    material.needsUpdate = true;
  }} catch (e) {{
    console.warn('Globe texture failed:', e);
  }}

  // Flat viewport
  const img = new Image();
  img.onload = () => {{
    flatImg = img;
    document.getElementById('file-size').textContent = `${{img.width}}\u00d7${{img.height}}`;
    drawFlat();
  }};
  img.onerror = () => {{
    ctx.fillStyle = '#222'; ctx.fillRect(0, 0, 560, 480);
    ctx.fillStyle = '#666'; ctx.font = '14px system-ui';
    ctx.fillText('Failed to load: ' + src, 20, 240);
  }};
  img.src = src;
}}

document.querySelectorAll('input[name="texture"]').forEach(r =>
  r.addEventListener('change', loadTexture)
);

document.getElementById('flat-lat').addEventListener('input', e => {{
  document.getElementById('flat-lat-val').textContent = parseFloat(e.target.value).toFixed(1) + '\u00b0';
  drawFlat();
}});
document.getElementById('flat-lon').addEventListener('input', e => {{
  document.getElementById('flat-lon-val').textContent = parseFloat(e.target.value).toFixed(1) + '\u00b0';
  drawFlat();
}});
document.getElementById('show-grid').addEventListener('change', e => gridGroup.visible = e.target.checked);
const autoRotate = document.getElementById('auto-rotate');

// \u2500\u2500 Render loop \u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500
(function animate() {{
  requestAnimationFrame(animate);
  if (autoRotate.checked) globe.rotation.y += 0.002;
  orbitCtrl.update();
  renderer.render(scene, camera);
}})();

loadTexture();
</script>
</body>
</html>
"""
    (TEXTURE_DIR / 'viewer.html').write_text(html)


def main() -> None:
    parser = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    parser.add_argument(
        '--port', type=int, default=8_091, help='HTTP server port (default: 8091)'
    )
    args = parser.parse_args()

    if not TEXTURE_DIR.exists():
        print('No textures found. Run texture_download.py first.')
        return

    generate_viewer()

    handler = functools.partial(
        http.server.SimpleHTTPRequestHandler, directory=str(TEXTURE_DIR)
    )
    server = http.server.HTTPServer(('127.0.0.1', args.port), handler)
    url = f'http://127.0.0.1:{args.port}/viewer.html'
    print(f'Serving at {url}')
    print('Press Ctrl+C to stop.\n')
    threading.Timer(0.5, lambda: webbrowser.open(url)).start()
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        print('\nStopped.')
        server.server_close()


if __name__ == '__main__':
    main()
