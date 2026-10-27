# WASM Widget Struct

A widget is a struct that implements `Widget`, with `#[widget]` on the impl block.

- The SDK owns the one instance: `init` builds it, every other hook runs as a method on it, and `unload` takes it by
  value as its last call.
- `#[widget]` exports `init`, `render`, `unload` and only the optional hooks the impl writes: a widget that leaves out
  `on_touch` gets no touches.
- A hook the impl leaves out cannot be exported by hand either: `#[widget]` holds its name, so a hand-written `on_touch`
  fails to compile for wasm32 on the duplicate symbol rather than run outside the instance.
- One `#[widget]` per crate: a second one fails to compile for wasm32 on the duplicate `init`.

```rust
use bmc_wasm_sdk::*;

struct RandomFact {
    fact: Option<String>,
    _poll: PollHandle,
}

impl RandomFact {
    fn build_request(&mut self, _poll: &PollHandle) -> Option<FetchSpec> {
        Some(FetchSpec::get(FACTS_API_URL))
    }

    fn on_fact_data(&mut self, poll: &PollHandle, response: &FetchResponse) {
        match response.ok().then(|| response.json().str("/data/text")).flatten() {
            Some(fact) => self.fact = Some(fact),
            None => poll.retry(),
        }
        request_frame();
    }
}

#[widget]
impl Widget for RandomFact {
    fn init(me: &Building<Self>) -> Self {
        let poll = PollHandle::register(me, Self::build_request, Self::on_fact_data, PollConfig::default());
        Self { fact: None, _poll: poll }
    }

    fn render(&mut self, _delta_ms: u32) {
        let fact = self.fact.as_deref().unwrap_or("Loading...");
        // …
    }

    fn on_system_update(&mut self) {
        request_frame();
    }
}
```

What runs when, for this widget:

1. The host calls `init`. The poll registers through `me`, but its first `build_request` waits: the instance does not
   exist until `init` returns.
2. `init` returns, the SDK stores the instance, and before the host gets control back `build_request` runs on it and the
   request goes out.
3. In a later host call the reply arrives, and `on_fact_data` runs on the instance.
4. A `poll.retry()` there only queues the next request. `on_fact_data` returns, then `build_request` runs and the
   request goes to the host, to be sent after `retry_ms`; only then does the export end and the host get control back.
   An `invalidate()` from a hook such as `on_params_update` runs the same way: the hook returns, `build_request` runs,
   the export ends.
5. `unload` drops the instance, and with it `_poll`, which ends the poll.

## Callbacks Are Methods

Every SDK call that takes a callback first takes what the callback runs on, its receiver; the callback is a method of
the receiver's type.

| Free function (obsolete)      | Receiver first                                               |
| ----------------------------- | ------------------------------------------------------------ |
| `timer::start`                | `Timer::start`                                               |
| `register_poll`               | `PollHandle::register`                                       |
| `request.send`, `send_after`  | `request.send_on`, `send_after_on`                           |
| `slot.set_fit`, `set_fit_ref` | `slot.set_fit_on`, `set_fit_ref_on`                          |
| `set_bitmap_fit`              | `set_bitmap_fit_on`                                          |
| `ws_connect`, `ws!`           | `Ws::connect`                                                |
| `tls_connect`, …              | `Socket::tls_connect`, `tls_connect_insecure`, `tcp_connect` |
| `mdns_browse`                 | `MdnsBrowse::start`                                          |
| `ssdp_search`                 | `SsdpSearch::start`                                          |
| `udp_broadcast`               | `UdpBroadcast::start`                                        |
| `http_listen`                 | `HttpListener::listen`                                       |

The receiver is whatever the calling code has at hand:

| Receiver    | Passed from                                                | The callback runs on        |
| ----------- | ---------------------------------------------------------- | --------------------------- |
| `self`      | a method of the widget                                     | the widget                  |
| `me`        | `init`, or the build of a part's `Part::new_cyclic`        | the value, once it is built |
| `&part`     | the widget, which holds the `Part<T>`                      | the part's value            |
| `&self.me`  | a part's value, through a `PartHandle` it keeps            | the part's value            |
| `self.me()` | a method of the widget, to code that runs on a field of it | the widget                  |

- The free `fetch` and `fetch_after` have no row: send a `FetchRequest` with `send_on` instead.
- The method takes `&mut self`, then the arguments the callback has always had. A method of a type other than the
  receiver's does not compile, nor does a part passing `self`:
  `` `Session` cannot receive callbacks for `Session`'s methods ``.
- Every form returns what its free function returns, with one addition: through a `PartHandle` whose part is dropped, it
  is refused. So `PollHandle::register` returns the handle itself, except through a `PartHandle`, where it returns an
  `Option`.
- A callback runs as one call into the widget, like a hook: on the one instance, with nothing else borrowing it, on the
  fuel budget of the host call that delivers it and under the import rules of the `on_*` hooks (the lifecycle guard
  matrix in the `bmc_wasm_sdk` crate docs).
- The free functions take a plain `fn` for a widget that keeps its state in `thread_local!` cells. They stay only so
  that a widget not yet ported still builds.

## Parts

A sub-object of the widget, a session or a screen say, owns its own timers and polls when it lives in a `Part<T>`.
`with` and `with_mut` lend its value for one call; dropping the part drops the value and the handles it holds.

```rust
use bmc_wasm_sdk::timer::Timer;

struct Session {
    hashrate: Option<f64>,
    sampler: Option<Timer>,
}

impl Session {
    fn new() -> Part<Self> {
        Part::new_cyclic(|me| Self {
            hashrate: None,
            sampler: Timer::start(me, 60_000, Self::sample),
        })
    }

    fn sample(&mut self, _timer: &Timer, _elapsed_periods: u32) {
        // …
    }
}

type Feed = Part<Session>;

struct Dashboard {
    feed: Option<Feed>,
}
```

- A callback runs as the value's method for as long as the part lives; once it is dropped, its registrations deliver
  nothing.
- The value reaches only its own fields and handles, not the widget. The widget reaches the value through the part, from
  its own methods: `feed.with(|value| value.hashrate)`.
- Name the part with a type alias, as `Feed` above, so fields name the sub-object rather than the box. Still construct
  it on the value's type, `Session::new()`: the alias's `new` is `Part::new`, which takes the value.

### A Part That Registers Its Own Callbacks

A chain such as a fetch whose reply starts a decode needs the value to name its own part after it is built. It keeps the
`PartHandle<T>` that `me.handle()` gives while it is built:

```rust
struct Loader {
    me: PartHandle<Loader>,
}

impl Loader {
    fn new() -> Part<Self> {
        Part::new_cyclic(|me| Self { me: me.handle() })
    }

    fn fetch(&mut self, url: &str) {
        let _ = FetchRequest::get(url).send_on(&self.me, Self::on_fetched);
    }

    fn on_fetched(&mut self, response: &FetchResponse) {
        let fit = Fit::Within { w: 64, h: 64 };
        let _ = set_bitmap_fit_on(&self.me, "art", response.body(), fit, b"", Self::on_decoded);
    }

    fn on_decoded(&mut self, _job: ImageJobId, _bitmap: Option<BitmapId>) {}
}
```

- The handle is weak: it never keeps the part alive. It counts as live only once the build has returned, so inside the
  build register through `me`, not the handle.
- Once the part is dropped, registering through the handle is refused: the form returns `None`, and
  `slot.set_fit_ref_on` hands the body back. No refusal says why: the same `None`, or the body handed back, also comes
  when the host itself refuses the request, such as a decode.
- A callback that drops its own part keeps the value until it returns; a registration it makes after the drop is
  accepted and delivers nothing.
- A sub-object whose callbacks must change the widget's state cannot be a part: the widget owns it as a plain field and
  runs its callbacks itself, passing `self.me()` to the field for the registrations it makes.

## Two Rules

**A part is not reached from its own callback.**

- While the value's method runs, the value is borrowed mutably, so the part refuses to lend it until the method returns.
- Ownership makes this hard to hit: the widget holds the part, and the part's method has no path to the widget. It takes
  a part shared outside the widget, in a `thread_local!` or an `Rc` its own value can reach.
- Reached that way, the part panics with `BUG: a Part reached from its own callback` rather than alias the value.

**The SDK never calls into the widget from inside a method.**

- A method callback runs from the top of a host-called export, never from inside an SDK call the widget made, so a
  method never has its own instance delivered to under it.
- SDK work that has to call back, such as a poll building its request after `enable_after` or `retry`, waits until the
  method that called `enable_after` or `retry` returns, then runs before the host gets control back, on that export's
  fuel budget. Of several requests a poll got by then, the last one wins, and `PollHandle::enabled()` in the same method
  reflects the request, not a `build` that has not run.
- A method-callback poll therefore needs an SDK export around it: drive it from `#[widget]` hooks or SDK callbacks. From
  a hand-written export nothing would run the queued `build`, so the SDK refuses with a `BUG:` panic. A poll registered
  with the free `register_poll` builds at once, as before.

## Traps

- A method that runs out of fuel or panics traps, and the wasm stack unwinds nothing. The trap can land mid-update, so
  the instance never runs again.
- The host tears the widget down and never calls into it again; the supervisor starts a new instance after its crash
  backoff. The value the trap left borrowed is never reached again, so the SDK keeps no state of its own about traps.
- Nothing of the trapped instance carries over, and its `unload` does not run.
- A trap outside any method, in a plain-function callback or poll `build` or in SDK code between methods, is no safer:
  it may have left the widget's own state or a poll half-updated, and it ends the instance the same way.
- A trap in `init` fails the widget's instantiation.
