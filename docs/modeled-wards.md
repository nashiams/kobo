# Modeled Ward Tutorial

Modeled wards are the Kobo surface for code whose behavior depends on history:
queues, gateways, sessions, leases, and other stateful flows. They collect the
state, ports, recordings, lifecycle obligations, scenarios, invariants, and
temporal checks that Kobo needs to turn a stateful failure into replayable
evidence.

The attribute form remains the stable base:

```rust
#[kobo::ward]
struct DurableQueue {}

#[kobo::must_call(ack, nack, requeue)]
struct Delivery {}

#[kobo::scenario(profile = "strict")]
fn crash_during_ack(queue: &mut DurableQueue) {
    let delivery = queue.recv();
    delivery.ack();
}
```

The first-class ward syntax is optional sugar over the same semantic model:

```text
ward DurableQueue {
    state log: Vec<String>
    obligation Delivery must ack | nack | requeue
    port storage: external
    recording storage_io from ".kobo/recordings/storage.krec"

    scenario crash_during_ack {
        let delivery = ward.recv();
        delivery.ack();
    }
}
```

Kobo must expose the same ward, scenario, obligation, port, recording, and debt
facts for both forms. A user can move between them without changing ordinary
runtime behavior for accepted code.

Ports name the external boundary the ward crosses. Recordings attach observed
boundary evidence to a scenario. Opaque boundaries are kept visible when the
toolchain cannot inspect an external surface; in user-facing reports they appear
as opaque boundaries. They produce debt or downgraded replay status instead of
silently pretending that the missing behavior was understood.

Every modeled fact should keep source spans. Diagnostics and witness events need
to point at the original obligation create site, terminal action, port, or
recording declaration rather than generated Rust.
