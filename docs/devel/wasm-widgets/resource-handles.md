# WASM Widget Resource Handles

Each SDK call in the table below starts something in the background and returns a handle to it. The handle is the only
one for that resource: it cannot be copied, and dropping it ends the resource. A free `FetchRequest::send` or an image
decode returns a plain id instead, and ends on its own.

| Call                                                         | Handle             | Ends it        |
| ------------------------------------------------------------ | ------------------ | -------------- |
| `Timer::start`                                               | `Timer`            | `stop()`       |
| `FetchRequest::send_on`, `send_after_on`                     | `Fetch`            | dropping it    |
| `Socket::tls_connect`, `tls_connect_insecure`, `tcp_connect` | `Socket`           | `close()`      |
| `Ws::connect`                                                | `Ws`               | `close()`      |
| `MdnsBrowse::start`                                          | `MdnsBrowse`       | `stop()`       |
| `mdns::mdns_register`                                        | `MdnsRegistration` | `unregister()` |
| `SsdpSearch::start`                                          | `SsdpSearch`       | `stop()`       |
| `UdpBroadcast::start`                                        | `UdpBroadcast`     | `stop()`       |
| `HttpListener::listen`                                       | `HttpListener`     | `close()`      |
| `PollHandle::register`                                       | `PollHandle`       | `stop()`       |

Each call but `mdns_register` takes a receiver first, the widget or a part, and one of its methods as the callback; its
free-function form (`timer::start`, `socket::tls_connect`, `register_poll`, …) takes a plain function, as does the `ws!`
macro, and stays only so that a widget not yet ported still builds. See [Widget Struct](widget-struct.md) for the
receivers.

The host may refuse every call but the poll registrations, such as one past the runtime limit: a receiver form then
returns `Err(RegisterError)`, or `Err(HandleError::Register)` through a `PartHandle`. A socket from `Ws::connect` can
still be refused after that, for a credential or a URL the host will not dial; it reports why at the next delivery as
`WsEvent::HostClosed`. The host logs why when a runtime limit refuses it, a listener's port cannot be bound, or the mDNS
daemon cannot register a service; input it cannot use, such as an empty list of service types, is refused without a log.
A hermetic capture refuses `HttpListener::listen` and `mdns_register`; the other network calls get a handle that
receives only the events in the capture's fixture, and none when the fixture has none. A widget's unit tests run
hermetic with no fixture: `http_listen`, `HttpListener::listen` and `mdns_register` are refused, and the other calls get
a handle that never receives an event. A call through the widget, `Timer::start(self, …)` say, panics instead, as a test
installs no widget; see [Unit Testing](unit-testing.md).

## Keep The Handle

A resource lives as long as its handle, unless the other end ends it first, so keep the handle for as long as the widget
wants the resource, typically in a field of the widget:

```rust
use bmc_wasm_sdk::mdns::{MdnsBrowse, MdnsEvent};
use bmc_wasm_sdk::*;

struct Finder {
    browse: Option<MdnsBrowse>,
}

impl Finder {
    fn on_mdns_event(&mut self, _browse: &MdnsBrowse, event: &MdnsEvent<'_>) {
        if let MdnsEvent::Found(json) = event {
            log_info!("found: {json}");
        }
    }
}

#[widget]
impl Widget for Finder {
    fn init(me: &Building<Self>) -> Self {
        let browse = MdnsBrowse::start(me, &["_http._tcp"], Self::on_mdns_event).ok();
        if browse.is_none() {
            log_warn!("mDNS browse rejected by host runtime limits");
        }
        Self { browse }
    }

    fn render(&mut self, _delta_ms: u32) { /* … */ }
}
```

Checking the result with `MdnsBrowse::start(...).is_err()` and keeping nothing compiles without a warning, and ends the
browse as soon as `init` returns. So does `let _ = MdnsBrowse::start(...)`, which is what the compiler suggests for an
unused result. The SDK offers no way to detach a handle and leave its resource running, so short of leaking the handle
(`std::mem::forget`, `ManuallyDrop`, etc.), every background resource a widget starts stays one it can stop.

## End A Resource

Call the method in the table, or drop the handle; both do the same. A `Fetch` has no method, so dropping it is the only
way. The method consumes the handle, so using it afterwards does not compile. An event for the resource already due in
the same delivery pass is dropped, so its callback never runs after the resource ends.

## Callbacks Borrow The Handle

A callback receives a borrowed handle, `&Socket` or `&Ws` for example, so it can use the resource but not end it. To end
the resource from its own callback, take the stored handle out of its field:

```rust
fn on_socket_event(&mut self, socket: &Socket, event: &SocketEvent<'_>) {
    match event {
        SocketEvent::Connected => socket.write(b"hello"),
        SocketEvent::Data(b"bye") => {
            if let Some(socket) = self.socket.take() {
                socket.close();
            }
        }
        SocketEvent::Data(_) => {}
        SocketEvent::Closed(_) => self.socket = None,
    }
}
```

The borrowed handle stays usable after that, but does nothing: a write or a send on a closed connection is dropped.

When the peer closes a socket or a WebSocket, the callback gets `SocketEvent::Closed` or `WsEvent::Close`, and the
stored handle reaches nothing from then on. A WebSocket the host closes, or refuses to open, gets `WsEvent::HostClosed`
instead, and ends the same way. Clear or replace it there, so a stored `Some` keeps meaning connected. A message over
the host's cap, 4 MiB by default (`max_websocket_message_bytes`), is one such close: the widget gets
`WsEvent::HostClosed(WsHostClose::MessageTooLarge)`, and the peer a 1009.

Several handles of one kind can share a callback. To find the one an event belongs to, compare the argument with the
stored handles.

## Poll Handles

`PollHandle::stop()` ends a poll, and so does dropping the handle. Either one cancels the poll's live fetch, a reply
already on its way is discarded, and the reply handler never runs again. `set_enabled(false)` only pauses a poll.

A poll registered with `PollHandle::register` builds once the method or callback that asked for the fetch has returned,
before the export ends, and of the fetches asked for by then the last one wins. A reply handler that calls
`enable_after` or `retry` on its handle therefore never sees `build` run under it. A poll registered with the free
`register_poll` builds at once. In both forms `build` may read its handle, may retry, may disable the poll, and may end
it by dropping the stored handle, in which case nothing is sent. It must not ask any poll for a request (`invalidate`,
`enable_after`, `set_enabled(true)`, or registering one): that would run a `build` again before this one's request goes
out, so the SDK panics instead.

`PollHandle::index` numbers polls in registration order, and an ended poll's index is never given to another poll, so
widget state keyed by it stays unambiguous. Each ended poll keeps an empty slot for the widget's lifetime, so a widget
that registers new polls over and over grows without bound.
