fn main() {
// kobo: config @ line 2 -> plain (local-only non-Copy binding)
    let config = make_config();
    process(config);
    log(config);
}
