use std::cell::RefCell;
use std::rc::Rc;
fn main() {
// kobo: config @ line 2 -> rc_refcell (mutable shared, last resort)
    let config = Rc::new(RefCell::new(make_config()));
    process(config.borrow());
    log(config.borrow());
}
