// Copyright (C) 2026  Braiins Systems s.r.o.

#[test]
fn widget_output_target_routes_to_widget_log_only() {
    let td = tempfile::tempdir().expect("BUG: tempdir");
    let bmc_log_path = td.path().join("bmc.log");
    let widget_log_path = td.path().join("widgets.log");

    // SAFETY: this is the only test in this binary, so no other thread
    // accesses the environment concurrently.
    unsafe { std::env::remove_var("RUST_LOG") };
    bmc_log::init_file_with_widget_capture(&bmc_log_path, &widget_log_path)
        .expect("BUG: init_file_with_widget_capture");

    tracing::info!(target: "widget_output", "weather[42]: hello from widget");
    tracing::info!("bmc internal event");

    let bmc_contents = std::fs::read_to_string(&bmc_log_path).expect("BUG: read bmc log");
    let widget_contents = std::fs::read_to_string(&widget_log_path).expect("BUG: read widget log");

    assert_eq!(bmc_log::WIDGET_OUTPUT_TARGET, "widget_output");
    assert!(widget_contents.contains("weather[42]: hello from widget"));
    assert!(!widget_contents.contains("bmc internal event"));
    assert!(bmc_contents.contains("bmc internal event"));
    assert!(!bmc_contents.contains("weather[42]"));
}
