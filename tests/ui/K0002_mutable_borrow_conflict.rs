use std::cell::RefCell;
use std::rc::Rc;
fn main() {
// kobo: data @ line 2 -> rc_refcell (mutable shared, last resort)
    let data = Rc::new(RefCell::new(vec![1, 2, 3]));
// kobo: r @ line 3 -> borrow-scope-conservative
    let r = &*data.borrow();
    data.borrow_mut().push(4);
// kobo: _len @ line 5 -> plain (local-only non-Copy binding)
    let _len = r.len();
}
