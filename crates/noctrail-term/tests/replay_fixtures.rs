use std::{fs, path::Path};

use noctrail_term::recording::replay_recording_file;

#[test]
fn replays_all_terminal_fixtures() {
    let fixtures_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("tests")
        .join("fixtures")
        .join("terminal");

    let mut fixtures = Vec::new();
    for entry in fs::read_dir(&fixtures_dir).expect("fixture directory") {
        let path = entry.expect("fixture entry").path();
        if path.extension().and_then(|ext| ext.to_str()) == Some("ntrec") {
            fixtures.push(path);
        }
    }

    fixtures.sort();
    assert!(
        !fixtures.is_empty(),
        "expected at least one terminal fixture in {}",
        fixtures_dir.display()
    );

    for fixture in fixtures {
        replay_recording_file(&fixture)
            .unwrap_or_else(|error| panic!("fixture {} failed: {error}", fixture.display()));
    }
}
