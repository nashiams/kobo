# Async Gateway Demo

This demo models request tokens with `reply`, `reject`, and `cancel` terminal
actions. The failing histories cover cancellation, orphan task, and
request-token mistakes under scheduler-sensitive execution. The passing history
shows handler inference and replayable witness output.

Useful commands:

```powershell
kobo test --sim deep --engine both --seed 1318 --witness-dir .kobo/witnesses --target cancellation_failure --inject cancel examples/async_gateway/cancellation_failure.kobo
kobo test --sim deep --engine both --seed 1318 --witness-dir .kobo/witnesses --target gateway_success examples/async_gateway/passing_history.kobo
kobo replay .kobo/witnesses/<witness>.kwit
kobo inspect --clean --cargo target/gateway-clean examples/async_gateway/clean_exit.kobo
```
