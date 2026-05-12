# Content-Addressed Texture Cache

Proposal for sharing GPU textures between widget processes via a compositor-managed cache.

## Problem

Each flip-clock widget creates identical digit textures (10 digits × 128×256×4 = 1.3 MB). With 5 widgets, that's 6.5 MB
of duplicated GPU memory for identical content.

## Solution

A content-addressed texture cache in the compositor, keyed by SHA-256 of pixel data. Widgets compute hash locally, then
lookup/store via Wayland protocol.

## Protocol Extension

```xml
<interface name="deck_texture_cache_v1" version="1">
  <description summary="Content-addressed texture cache">
    Shared texture cache keyed by SHA-256 of pixel data.
    Widgets compute hash locally, then lookup/store.
  </description>

  <request name="lookup">
    <description summary="lookup texture by content hash">
      Check if texture exists in cache. Compositor responds with
      hit (fd returned) or miss.
    </description>
    <arg name="hash" type="array" summary="32-byte SHA-256"/>
  </request>

  <request name="store">
    <description summary="store texture in cache">
      Store rendered texture. Compositor verifies hash matches
      content, then caches. Returns fd for the cached copy.
    </description>
    <arg name="hash" type="array" summary="32-byte SHA-256"/>
    <arg name="dmabuf" type="fd"/>
    <arg name="width" type="uint"/>
    <arg name="height" type="uint"/>
    <arg name="stride" type="uint"/>
    <arg name="format" type="uint" summary="DRM fourcc"/>
  </request>

  <event name="hit">
    <arg name="hash" type="array"/>
    <arg name="dmabuf" type="fd"/>
    <arg name="stride" type="uint"/>
  </event>

  <event name="miss">
    <arg name="hash" type="array"/>
  </event>
</interface>
```

## Flow

```
Widget A (first flip-clock)          Compositor                     Widget B (second flip-clock)
─────────────────────────           ──────────                     ──────────────────────────

render digit "5" locally
hash = sha256(pixels)
lookup(hash)              ────►
                          ◄────     miss(hash)
store(hash, fd, ...)      ────►
                                    verify hash matches content
                                    cache.insert(hash, texture)
                          ◄────     hit(hash, cached_fd)

                                                                    render digit "5" locally
                                                                    hash = sha256(pixels)  [same hash]
                                                                    lookup(hash)           ────►
                                                                                     ◄────  hit(hash, fd)
                                                                    import fd, skip upload
```

## Widget-Side Implementation

```rust
impl TextureCacheClient {
    fn get_or_create(&mut self, pixels: &[u8], width: u32, height: u32) -> Texture {
        let hash = sha256(pixels);

        self.protocol.lookup(&hash);
        match self.protocol.recv() {
            CacheEvent::Hit { fd, stride, .. } => {
                self.import_dmabuf(fd, width, height, stride)
            }
            CacheEvent::Miss { .. } => {
                let texture = self.upload_pixels(pixels, width, height);
                let fd = self.export_dmabuf(&texture);
                self.protocol.store(&hash, fd, width, height, stride, format);
                texture
            }
        }
    }
}
```

## Compositor-Side Implementation

```rust
struct TextureCache {
    cache: HashMap<[u8; 32], CachedTexture>,
}

struct CachedTexture {
    texture: GlesTexture,
    dmabuf: Dmabuf,
    width: u32,
    height: u32,
    stride: u32,
    format: DrmFourcc,
    refcount: usize,
}

impl TextureCache {
    fn lookup(&self, hash: &[u8; 32]) -> Option<&CachedTexture> {
        self.cache.get(hash)
    }

    fn store(&mut self, hash: [u8; 32], dmabuf: Dmabuf, ...) -> Result<&CachedTexture> {
        if let Some(existing) = self.cache.get_mut(&hash) {
            existing.refcount += 1;
            return Ok(existing);
        }

        let texture = self.renderer.import_dmabuf(&dmabuf)?;
        self.cache.insert(hash, CachedTexture { texture, dmabuf, refcount: 1, ... });
        Ok(self.cache.get(&hash).unwrap())
    }
}
```

## Benefits

- **Universal**: Works for any texture content, not just fonts
- **Automatic dedup**: Identical pixels from different sources share storage
- **Verifiable**: Compositor can verify hash matches content
- **Simple mental model**: Like git blobs or IPFS
- **No schema**: No need to agree on key formats

## Memory Savings

| Scenario                   | Without Cache       | With Cache | Savings       |
| -------------------------- | ------------------- | ---------- | ------------- |
| 5 flip-clocks, same digits | 5 × 1.3 MB = 6.5 MB | 1.3 MB     | 5.2 MB (80%)  |
| 10 flip-clocks             | 10 × 1.3 MB = 13 MB | 1.3 MB     | 11.7 MB (90%) |

## Considerations

### Hash Verification

Compositor should verify that stored content matches the claimed hash to prevent cache poisoning. This requires reading
back the DMA-BUF content, which has a CPU cost.

Alternative: trust widgets (same security domain) and skip verification.

### Eviction Policy

Options:

- LRU with size limit
- Reference counting (evict when refcount drops to 0)
- TTL-based expiry
- Hybrid: keep referenced textures, LRU for unreferenced

### First-Render Cost

The first widget to need a texture always renders locally to compute the hash. This is unavoidable with true
content-addressing. Subsequent widgets benefit from cache hits.

### Startup Optimization

For predictable content (digit glyphs), widgets could:

1. Batch-lookup all 10 digit hashes at startup
2. Render only cache misses
3. Store rendered textures

This requires computing hashes without rendering, which means hashing the "recipe" (font + codepoint + size) instead of
pixels. Trade-off: less universal, but faster warm start.

## Future Extensions

- **Persistent cache**: Survive compositor restart by storing to disk
- **Texture atlas packing**: Combine small textures into shared atlases
- **Compression**: Store compressed, decompress on GPU
- **Metrics**: Track hit rate, memory saved, popular textures
