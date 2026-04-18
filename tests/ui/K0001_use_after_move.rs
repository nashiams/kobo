fn main() {
// kobo: config @ line 2 -> plain (moved then dead (freeze-and-rotate))
    let config = make_config();
    process(config);
    log(config);
}
