# DREAD wire protocol, version 1

All integers are **big-endian**. Coordinates are normalized to `0..=65535`
across the streamed frame (0 = left/top edge, 65535 = right/bottom edge).

## Transports

| Transport | How channels are carried |
|---|---|
| Wi-Fi / LAN | Two TCP connections to the server port (default `47800`), each wrapped in TLS 1.3. The first is the **control** channel, the second the **video** channel. |
| USB (AOA) | One bulk IN + one bulk OUT endpoint. Multiplexed as `[u8 channel][u32 length][bytes]`, channel `0` = control, `1` = video. A zero-length payload closes that channel (either direction); the server then waits for a new `hello` on it, so a client can reconnect without re-plugging. No TLS: the link is a physical cable that the user approved on both ends; the device token is still required, and code-comparison pairing uses the `fp` sent in `pair_required`. |

Discovery: mDNS service `_dread._tcp.local.` (TXT: `id`, `name`, `fp`,
`proto`) and a UDP fallback — the client broadcasts `DREAD?` to port
`47801`; servers answer with a JSON object
`{"t":"announce","id","name","port","fp","proto"}`.

## Framing

Every message on a channel:

```
u32 length   (of kind + payload, max 16 MiB)
u8  kind     0x01 = JSON (UTF-8), 0x02 = INPUT (binary), 0x03 = VIDEO (binary)
... payload
```

## Handshake (control channel)

```
C → S  {"t":"hello","proto":1,"role":"control","device_id":"<uuid>",
        "device_name":"Pixel 8","token":"<base64>|null",
        "client":{"width":2400,"height":1080,"density_dpi":420,
                  "max_fps":120,"codecs":["h264","hevc","av1"],
                  "touch_points":10,"stylus":true}}
```

If `token` is valid:

```
S → C  {"t":"welcome","session":"<hex32>","video_key":"<hex64>",
        "server_name":"workstation","server_id":"<uuid>"}
```

Otherwise `S → C {"t":"pair_required","methods":["qr","sas"]}` and the
client runs one of the pairing flows below.

### QR pairing

The QR code encodes
`dread://pair?v=1&h=<ip>[,<ip>...]&p=<port>&fp=<sha256 hex>&s=<secret b64url>&n=<name>`.
The client connects, checks that the server certificate fingerprint equals
`fp`, then sends `{"t":"pair_qr","secret":"<b64url>"}`.

### Code-comparison pairing

```
C → S  {"t":"pair_commit","commit":hex(SHA256(nc))}       nc = 32 random bytes
S → C  {"t":"pair_nonce","ns":hex(ns)}                     ns = 32 random bytes
C → S  {"t":"pair_reveal","nc":hex(nc)}
```

Both sides compute
`code = u32be(SHA256("DREAD-SAS-v1" || fp || nc || ns)[0..4]) mod 1_000_000`
(`fp` = 32-byte certificate hash *as seen by each side*) and display it
zero-padded to six digits. The server asks the user to approve.

Result of either flow: `{"t":"paired","token":"<base64 32 bytes>"}` followed
by `welcome`, or `{"t":"pair_rejected","reason":"..."}`.

## Video channel

```
C → S  {"t":"hello","proto":1,"role":"video","session":"<hex>","video_key":"<hex>"}
S → C  {"t":"video_ok"}
S → C  VIDEO frames
```

VIDEO payload:

```
u8  flags        bit0 = key frame, bit1 = contains codec config (SPS/PPS/VPS/seq header)
u32 seq          increments per frame, resets with every stream_config
u64 capture_us   server monotonic clock at capture
u32 encode_us    capture → encoded
... access unit  (Annex-B for H.264/HEVC, low-overhead OBUs for AV1)
```

## Control messages

Server → client:

| `t` | fields | meaning |
|---|---|---|
| `stream_config` | `codec`, `width`, `height`, `fps`, `bitrate`, `generation` | sent **on the video channel**, in order with the frames: (re)configure the decoder; the next frame is a key frame |
| `pong` | `ts`, `server_us` | reply to ping |
| `ime` | `show` (bool) | a text field on the DREAD display gained/lost focus |
| `identify` | `name`, `width`, `height` | show the display name on the device for 3 s |
| `stats` | `fps`, `bitrate`, `encode_ms`, `encoder`, ... | for the client overlay |
| `display_info` | `mode` (`extend`/`duplicate`), `name`, `width`, `height` | current display |
| `bye` | `reason` | graceful disconnect |

Client → server:

| `t` | fields | meaning |
|---|---|---|
| `ping` | `ts`, `rtt_ms` | heartbeat every second (`rtt_ms` = last measured round trip); server drops the session after the configured timeout (default 6 s) |
| `ack` | `seq`, `gen`, `decode_us` | frame decoded and rendered (drives flow control and latency stats) |
| `request_keyframe` | | decoder lost sync |
| `viewport` | `width`, `height`, `rotation` | client surface changed (rotation, split-screen); used by automatic resolution |
| `bye` | | graceful disconnect |

## INPUT payloads

```
u8 type
```

| type | name | body |
|---|---|---|
| `0x01` | TOUCH | `u8 action` (0 down, 1 move, 2 up, 3 cancel), `u8 changed_id`, `u8 n`, then `n` × `{u8 id, u16 x, u16 y, u16 pressure, u16 major}` — the full set of contacts after the event (an `up` contact is still listed in its own event) |
| `0x02` | PEN | `u8 tool` (0 pen, 1 eraser), `u8 flags` (bit0 in-range, bit1 contact, bit2 barrel button 1, bit3 barrel button 2), `u16 x`, `u16 y`, `u16 pressure`, `i16 tilt_x`, `i16 tilt_y` (hundredths of a degree) |
| `0x03` | MOUSE_ABS | `u16 x`, `u16 y`, `u8 buttons` (bit0 left, bit1 right, bit2 middle) |
| `0x04` | MOUSE_REL | `i16 dx`, `i16 dy`, `u8 buttons` |
| `0x05` | SCROLL | `i16 vertical`, `i16 horizontal` (units of 1/120 notch, positive = down/right) |
| `0x06` | KEY | `u16 code` (Linux evdev key code), `u8 down` |
| `0x07` | TEXT | UTF-8 text to type |

Touch and pen contacts are delivered to the OS as real multitouch and real
pen events (pressure, tilt, eraser, buttons, hover) — never as emulated mouse
events.
