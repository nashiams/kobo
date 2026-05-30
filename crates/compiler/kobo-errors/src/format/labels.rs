pub(super) fn public_label_text(text: &str) -> String {
    text.replace("delivery-open", "this delivery still needs an ending")
        .replace("reply-open", "this reply still needs an ending")
        .replace(
            "obligation-left-open",
            "this obligation still needs an ending",
        )
        .replace("node id", "internal location")
        .replace("KoboSpan", "source span")
}
