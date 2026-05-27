use quote::ToTokens;
use syn::parse_quote;

use super::{
    captured_bindings_need_spawn_local, executor_main_attr, is_executor_main_attr,
    wrap_function_body_in_local_set,
};
use crate::executor::ExecutorChoice;
use crate::lower::rewrite::clone_inject::CapturedBinding;
use kobo_ir::OwnershipTier;

fn parsed_executor_attr(choice: ExecutorChoice) -> syn::Attribute {
    executor_main_attr(choice, true).expect("executor attribute should exist")
}

#[test]
fn executor_attr_detector_matches_supported_executors() {
    assert!(is_executor_main_attr(&parsed_executor_attr(
        ExecutorChoice::Tokio
    )));
    assert!(is_executor_main_attr(&parsed_executor_attr(
        ExecutorChoice::AsyncStd
    )));
}

#[test]
fn executor_attr_detector_ignores_other_attributes() {
    let attr: syn::Attribute = parse_quote!(#[allow(dead_code)]);
    assert!(!is_executor_main_attr(&attr));
}

#[test]
fn executor_attr_renders_expected_tokens() {
    let attr = parsed_executor_attr(ExecutorChoice::Tokio);
    assert_eq!(attr.to_token_stream().to_string(), "# [tokio :: main]");
}

#[test]
fn local_set_wrapper_for_async_function_uses_run_until_await() {
    let mut function: syn::ItemFn = parse_quote! {
        async fn run_local() -> u32 {
            tokio::task::spawn_local(async move {});
            7
        }
    };

    wrap_function_body_in_local_set(&mut function);
    let rendered = function.to_token_stream().to_string();

    assert!(rendered.contains("tokio :: task :: LocalSet :: new"));
    assert!(rendered.contains("run_until"));
    assert!(rendered.contains(". await"));
    assert!(rendered.contains("spawn_local"));
    assert!(rendered.contains("7"));
}

#[test]
fn local_set_wrapper_for_sync_function_uses_runtime_handle() {
    let mut function: syn::ItemFn = parse_quote! {
        fn run_local() {
            tokio::task::spawn_local(async move {});
        }
    };

    wrap_function_body_in_local_set(&mut function);
    let rendered = function.to_token_stream().to_string();

    assert!(rendered.contains("tokio :: task :: LocalSet :: new"));
    assert!(rendered.contains("tokio :: runtime :: Handle :: current"));
    assert!(rendered.contains("block_on"));
    assert!(rendered.contains("spawn_local"));
}

#[test]
fn captured_rc_tier_requests_spawn_local() {
    let captured = vec![CapturedBinding {
        name: "world".to_owned(),
        tier: OwnershipTier::RcMutShared,
        is_copy: false,
        used_after_spawn: true,
    }];

    assert!(captured_bindings_need_spawn_local(&captured));
}

#[test]
fn captured_arc_tier_keeps_tokio_spawn() {
    let captured = vec![CapturedBinding {
        name: "state".to_owned(),
        tier: OwnershipTier::ArcMutShared,
        is_copy: false,
        used_after_spawn: true,
    }];

    assert!(!captured_bindings_need_spawn_local(&captured));
}
