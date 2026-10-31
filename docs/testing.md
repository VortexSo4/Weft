# Testing Weft

## Unit tests

```sh
cargo test --workspace
```

These are ordinary Rust tests inside the engine, the SDK and the noise crate: terrain generation,
light propagation, the network codec, text layout, component tables and the settings parser. They
need no GPU and no game window.

## The in-game suite

Most of Weft is a running game, so most of its tests run inside one. `mods/testkit` boots a real
client, drives it frame by frame and talks to the real server.

```sh
cargo run --release -- --test all
cargo run --release -- --test list
cargo run --release -- --test furnace,armor
cargo run --release -- --test bench
```

`--test all` runs every test, each in a fresh throwaway world. `--test list` prints their names,
`--test a,b` runs some of them and `--test bench` runs the benchmarks instead. `--test-keep` keeps
the worlds afterwards.

The run writes:

- `run/test-reports/report.txt`: the result of every test and what it noted.
- `run/test-shots/`: screenshots of the run, for looking at.

The process exits with the number of failed tests, which is what CI would check. It needs a GPU
with Vulkan, since it is the actual renderer.

Tests never compare screenshots with reference pictures; pictures are written for a person to look
at, and the checks are numeric (pixel averages and shares, block states, inventory contents,
health, positions).

## Scripts

A line script drives one world without writing a test:

```sh
cargo run --release -- --world scratch --script my_script.txt
```

The instructions are in `mods/testkit/src/script.rs`. This is the quickest way to reproduce
something you saw while playing.

## Writing a test

Tests live in `mods/testkit/src/tests/`, grouped by area (`world.rs`, `gameplay.rs`, `ui.rs`,
`visual.rs`, ...). A test is an `async fn(T) -> R`:

```rust
pub async fn my_thing(t: T) -> R {
    t.set([0, 40, 0], "base:stone").await?;
    t.check_eq(t.block([0, 40, 0]).await?, "base:stone".to_string(), "the block is there");
    Ok(())
}
```

Add it to the `crate::tests![...]` list in `mods/testkit/src/tests/mod.rs`.

Things the runner does for you:

- `t.until(name, timeout, condition)` waits for the game to reach a state instead of sleeping.
  Every wait has a timeout, and a failed wait names itself in the report.
- `t.check(...)`, `t.check_eq(...)` and `t.note(...)` record results without stopping the test;
  `?` stops it.
- Waiting for the server is explicit: cubes and menus travel over the network, so wait for what
  they change rather than for a number of frames.
- The client and the server are both reachable (`t.c()` and the `/tk` server commands), which is
  how the sync tests compare the two.

## Translations

```sh
python3 scripts/check_lang.py
```

It reports keys a translation uses that `en_us` does not have (always an error, since nothing shows
them) and keys it has not translated yet (a warning, because the engine falls back to English). Use
`--strict` to treat untranslated keys as errors too.

## Checks before a change

The release workflow only builds the game for Windows on a tag; it does not run tests, clippy or
rustfmt. Run what applies to your change yourself:

```sh
cargo test --workspace
cargo run --release -- --test all
python3 scripts/check_lang.py
```

Two things to know about the tree as it is: `cargo clippy --workspace` reports lints that predate
your change, so compare against a clean checkout instead of expecting silence, and the sources are
not rustfmt-formatted, so do not run `cargo fmt --all` over the whole tree in one change.
