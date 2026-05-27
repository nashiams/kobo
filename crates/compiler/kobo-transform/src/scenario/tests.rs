use kobo_parser::parse_file;

#[test]
fn infers_normal_lifecycle_templates_from_scenario_ast() {
    let source = r#"
struct Queue {}
struct Delivery {}
struct Db {}
struct Tx {}
struct Request {}

impl Queue {
    fn recv(&self) -> Delivery { Delivery {} }
}

impl Delivery {
    fn ack(self) {}
    fn nack(self) {}
    fn requeue(self) {}
}

impl Db {
    fn begin(&self) -> Tx { Tx {} }
}

impl Tx {
    fn commit(self) {}
    fn rollback(self) {}
}

#[kobo::handler]
#[kobo::scenario(profile = "async")]
async fn service(request: Request) {
    let message = Queue {}.recv().await;
    message.ack();

    let tx = Db {}.begin();
    tx.commit();

    let task = tokio::spawn(async {});
    task.abort();

    request.reply();
}
"#;
    let mut id_gen = NodeIdGen::new();
    let ast = parse_file(source, FileId(0), &mut id_gen).expect("parse should succeed");
    let program = build_scenario_programs(&ast, &[], "async")
        .into_iter()
        .find(|program| program.target == "service")
        .expect("service scenario should lower");

    let creates = program
        .operations
        .iter()
        .filter_map(|operation| match &operation.kind {
            ScenarioOpKind::CreateObligation {
                binding,
                type_name,
                actions,
                ..
            } => Some((binding.as_str(), type_name.as_str(), actions.clone())),
            _ => None,
        })
        .collect::<Vec<_>>();
    for (binding, type_name, actions) in [
        ("request", "HandlerReply", handler_reply_actions()),
        ("message", "Delivery", queue_delivery_actions()),
        ("tx", "Transaction", transaction_actions()),
        ("task", "SpawnedTask", spawned_task_actions()),
    ] {
        assert!(
            creates
                .iter()
                .any(|candidate| candidate == &(binding, type_name, actions.clone())),
            "missing inferred obligation {binding}/{type_name}/{actions:?} in {creates:?}"
        );
    }

    let discharges = program
        .operations
        .iter()
        .filter_map(|operation| match &operation.kind {
            ScenarioOpKind::Discharge { binding, action } => {
                Some((binding.as_str(), action.as_str()))
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    for discharge in [
        ("request", "reply"),
        ("message", "ack"),
        ("tx", "commit"),
        ("task", "abort"),
    ] {
        assert!(
            discharges.contains(&discharge),
            "missing lifecycle discharge {discharge:?} in {discharges:?}"
        );
    }
}
