fn main() {
// kobo: data @ line 2 -> plain (local-only non-Copy binding)
    let mut data = vec![1, 2, 3];
    let r = &data;
    data.push(4);
// kobo: _len @ line 5 -> plain (local-only non-Copy binding)
    let _len = r.len();
}
