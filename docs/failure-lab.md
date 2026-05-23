# Failure Lab

Failure Lab is the user workflow for turning known history-sensitive bugs into
repeatable Kobo evidence. The current flagship cases are durable queue and async
gateway because they combine external boundaries, lifecycle obligations, and
scheduler-sensitive histories.

## Durable Queue

The durable queue demo models a delivery token with the terminal actions `ack`,
`nack`, and `requeue`. The failure history covers crash-after-ack behavior:

1. receive a delivery from the storage-backed queue;
2. acknowledge it;
3. crash before the storage facade has recorded the final durable state;
4. replay the `.kwit` witness and inspect the invariant or temporal failure.

The passing history records the storage facade, the port, the recording, and the
clean terminal action ordering. The failing history reports the source span for
the queue scenario and the event that violated the check.

## Async Gateway

The async gateway demo models request tokens with `reply`, `reject`, and
`cancel` terminal actions. Scheduler presets exercise cancellation, preemption,
orphan task, and request-token paths:

1. cancellation before a reply must consume or reject the request token;
2. preemption must not leave an orphan task carrying the obligation;
3. request-token diagnostics must identify the create site and missing terminal;
4. replayed witnesses keep scheduler seed and boundary evidence together.

Both demos should be runnable through public commands: `kobo test`, `kobo
replay`, `kobo inspect --clean --cargo`, and `kobo debt --summary`. The demos
are product checks, not private fixtures hidden behind unit-only helpers.
