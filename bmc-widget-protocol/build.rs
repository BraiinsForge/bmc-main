// Copyright (C) 2026  Braiins Systems s.r.o.

fn main() {
    // Regenerate bindings on changes.
    println!("cargo:rerun-if-changed=protocol/deck-widget-v1.xml");
}
