# Durable Queue Demo

This demo models a storage-backed delivery token with `ack`, `nack`, and
`requeue` terminal actions. Use the failing history to produce a crash-after-ack
witness, the passing history to verify inferred lifecycle actions, and the
metadata file to inspect ports, recordings, and debt.

Useful commands:

```powershell
kobo test --sim deep --engine both --seed 1317 --witness-dir .kobo/witnesses --target crash_after_ack examples/durable_queue/crash_after_ack.kobo
kobo replay .kobo/witnesses/<witness>.kwit
kobo inspect --clean --cargo target/durable-clean examples/durable_queue/clean_exit.kobo
kobo inspect --scenario-metadata examples/durable_queue/metadata.kobo
```
