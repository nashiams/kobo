# Editor Workflow

`kobo-lsp` is the editor entry point for the modeled-ward workflow. A normal
editor session should provide diagnostics, hovers, explain actions, witness
links, runnables, and Rust navigation delegation from source-mapped compiler
facts.

The expected loop is:

1. open a `.kobo` or Rust-shaped Kobo source file;
2. receive diagnostics with Kobo codes such as `K0100`;
3. use hover or explain actions to inspect the obligation and source span;
4. run the scenario through a runnable command;
5. follow witness links into `.kobo/witnesses`;
6. use rust-analyzer delegation and source maps for generated Rust navigation.

The VS Code extension registers syntax, language configuration, and the run,
replay, and explain commands. The editor should not invent a generated-only
location when a source span is missing; it should report the missing mapping as
debt or a proof failure.

## Clean Rust Exit Ramp

`kobo inspect --clean` prints Rust-shaped output without Kobo attributes. `kobo
inspect --clean --cargo <dir>` writes a Cargo project that should build without
Kobo runtime dependencies for ordinary accepted code. This is the zero Kobo dependency
exit ramp.

Rust-shaped formatting is delegated to rustfmt. Kobo-only syntax uses Kobo's
formatter only for the syntax that rustfmt cannot parse, while preserving source
mapping for diagnostics and editor actions.
