mod project_prose;

use crate::{diagnostic_registry, DiagnosticRegistryEntry};

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum ExplainDetail {
    Human,
    Verbose,
}

pub fn explain_code(code_text: &str) -> Option<String> {
    explain_code_with_detail(code_text, ExplainDetail::Human)
}

pub fn explain_code_with_detail(code_text: &str, detail: ExplainDetail) -> Option<String> {
    let registry = diagnostic_registry();
    registry
        .find_by_code_text(code_text)
        .map(|entry| match detail {
            ExplainDetail::Human => render_human_explain_entry(entry),
            ExplainDetail::Verbose => render_verbose_explain_entry(entry),
        })
}

pub fn unknown_code_message(code_text: &str) -> String {
    let registry = diagnostic_registry();
    match registry.nearest_code_text(code_text) {
        Some(nearest) => {
            format!("unknown diagnostic code `{code_text}`\nnearest registered code: {nearest}")
        }
        None => format!("unknown diagnostic code `{code_text}`"),
    }
}

fn render_human_explain_entry(entry: &DiagnosticRegistryEntry) -> String {
    let mut rendered = format!("{}: {}\n\n", entry.code_text, entry.title);

    push_inline_section(&mut rendered, "I found:", entry.summary);
    push_inline_section(&mut rendered, "Why I care:", entry.explain);
    push_section(&mut rendered, "Try this:", fix_guidance(entry));
    if let Some(example) = example_guidance(entry) {
        push_section(&mut rendered, "Example:", example);
    }
    push_inline_section(
        &mut rendered,
        "Hint:",
        &format!(
            "Run `kobo explain {} --verbose` for policy and machine-edit metadata.",
            entry.code_text
        ),
    );

    rendered
}

fn render_verbose_explain_entry(entry: &DiagnosticRegistryEntry) -> String {
    format!(
        "{} - {}\nslug: {}\ncategory: {}\nstatus: {}\nseverity: {}\nmode policy: {}\nsuggestion policy: {}\nmachine edits: {}\n\n{}\n",
        entry.code_text,
        entry.title,
        entry.slug,
        entry.category.as_str(),
        entry.status.as_str(),
        entry.default_severity.as_str(),
        entry.mode_behavior.as_str(),
        entry.suggestion_policy.as_str(),
        entry.machine_edit_policy.as_str(),
        entry.explain,
    )
}

fn push_section(rendered: &mut String, heading: &str, body: &str) {
    let body = body.trim();
    if body.is_empty() {
        return;
    }

    ensure_section_gap(rendered);
    rendered.push_str(heading);
    rendered.push('\n');
    rendered.push_str(body);
    rendered.push_str("\n\n");
}

fn push_inline_section(rendered: &mut String, heading: &str, body: &str) {
    let body = body.trim();
    if body.is_empty() {
        return;
    }

    ensure_section_gap(rendered);
    rendered.push_str(heading);
    rendered.push(' ');
    rendered.push_str(body);
    rendered.push_str("\n\n");
}

fn ensure_section_gap(rendered: &mut String) {
    if rendered.is_empty() || rendered.ends_with("\n\n") {
        return;
    }
    if !rendered.ends_with('\n') {
        rendered.push('\n');
    }
    rendered.push('\n');
}

#[derive(Copy, Clone)]
struct ExplainProse {
    fix: &'static str,
    example: &'static str,
}

fn fix_guidance(entry: &DiagnosticRegistryEntry) -> &'static str {
    if entry.status == crate::DiagnosticStatus::Reserved {
        return reserved_fix_guidance();
    }

    if let Some(prose) = explain_prose(entry.code) {
        return prose.fix;
    }

    match entry.code {
        crate::KErrorCode::K0001 => {
            "Want to use this value again after sending it somewhere?\n\
             - Borrow it if the other function only needs to read it.\n\
             - Clone only the small part you still need.\n\
             - Move the later use before the call that takes the value."
        }
        crate::KErrorCode::K0002 => {
            "Want to change this value while something else reads it?\n\
             - End the earlier read before the mutation.\n\
             - Move the mutation into a later scope.\n\
             - Borrow a smaller field when only that field must change."
        }
        crate::KErrorCode::K0025 => {
            "- Remove the hint when the requested ownership shape is not important.\n- Change the code so the value has only one owner."
        }
        crate::KErrorCode::K0041 => {
            "- Finish using or drop the aliases before entering the @strict block.\n- Keep this code outside strict when an alias must stay live."
        }
        crate::KErrorCode::K0042 => {
            "- Move the closure outside the @strict block.\n- Pass plain data into the closure so it does not capture the guarded value."
        }
        crate::KErrorCode::K0062 => {
            "- Add tokio or async-std to [dependencies].\n- Keep this code synchronous until runtime setup is explicit."
        }
        crate::KErrorCode::K0063 => {
            "- Use `@strict async fn` if the whole function should follow Kobo's async strict rules.\n- Move this strict work into a small non-async helper."
        }
        crate::KErrorCode::K0080 => {
            "- Choose an explicit ownership design before migration.\n- Record reviewed debt when the architecture decision must wait."
        }
        crate::KErrorCode::K0100 => {
            "Want to finish this obligation here?\n\
             - Call ack, nack, requeue, commit, rollback, or another required action.\n\
             - Pass the obligation to a helper that always completes it.\n\
             - Use debt(...) only when cleanup happens outside this scenario."
        }
        crate::KErrorCode::K0102 => {
            "Want this path to replay the same way later?\n\
             - Route time, randomness, scheduling, and IO through a modeled facade.\n\
             - Record the value in the witness.\n\
             - Mark replay debt when exact replay is not claimed."
        }
        crate::KErrorCode::K0107 => {
            "- Choose model, record, or outside when this path should stay replayable.\n- Choose opaque or debt when the boundary is accepted but not fully modeled."
        }
        crate::KErrorCode::K0108 => {
            "- Keep the suppression reason specific and reviewable.\n- Replace the suppression with a model or boundary policy when the path becomes covered."
        }
        _ => match entry.suggestion_policy {
            crate::SuggestionPolicy::MachineApplicableAllowed => {
                "Apply a machine-applicable suggestion only after checking that it preserves the source intent."
            }
            crate::SuggestionPolicy::BoundaryPolicy => {
                "Choose an explicit boundary policy so the guarantee remains reviewable."
            }
            crate::SuggestionPolicy::HelpOnly => {
                "Follow the help text from the diagnostic card and rerun Kobo."
            }
            crate::SuggestionPolicy::ReviewOnly
            | crate::SuggestionPolicy::None
            | crate::SuggestionPolicy::Unspecified => {
                "Review the highlighted source and make the ownership or boundary choice explicit."
            }
        },
    }
}

fn example_guidance(entry: &DiagnosticRegistryEntry) -> Option<&'static str> {
    if entry.status == crate::DiagnosticStatus::Reserved {
        return Some(reserved_example_guidance());
    }

    if let Some(prose) = explain_prose(entry.code) {
        return Some(prose.example);
    }

    match entry.code {
        crate::KErrorCode::K0001 => Some(
            "Problem:\nlet user = load_user();\nsend(user);\nlog(user.name);\n\nFix:\nBorrow for the send call, log before the move, or clone only the field that must remain available.",
        ),
        crate::KErrorCode::K0002 => Some(
            "Problem:\nlet name = &profile.name;\nprofile.name.push_str(\"!\");\nprintln(name);\n\nFix:\nEnd the read before the mutation, or move the mutation into a later block after the read is done.",
        ),
        crate::KErrorCode::K0025 => Some(
            "Problem:\n#[kobo::hint(ownership = \"move\")]\nlet buf = String::new();\nuse_mut(&mut buf);\nuse_mut(&mut buf);\n\nFix:\nRemove the hint, or refactor so `buf` has only one owner.",
        ),
        crate::KErrorCode::K0041 => Some(
            "Problem:\nlet alias = cache.view();\n@strict { cache.update(); }\nuse(alias);\n\nFix:\nFinish using or drop the alias before entering the @strict block.",
        ),
        crate::KErrorCode::K0042 => Some(
            "Problem:\n@strict {\n    let bump = || cache.update();\n    save(bump);\n}\n\nFix:\nMove the closure outside the strict block, or pass plain data into the closure instead of capturing the guarded value.",
        ),
        crate::KErrorCode::K0062 => Some(
            "Problem:\nasync fn main() {\n    handle().await;\n}\n\nFix:\nAdd a supported executor dependency such as tokio or async-std, or keep the entry point synchronous until runtime setup is explicit.",
        ),
        crate::KErrorCode::K0063 => Some(
            "Problem:\nasync fn handler() {\n    @strict { data.borrow_mut().push(1); }\n    wait().await;\n}\n\nFix:\nUse `@strict async fn` if the whole function should follow Kobo's async strict rules, or move this strict work into a small non-async helper.",
        ),
        crate::KErrorCode::K0080 => Some(
            "Problem:\nstate is mutated from several unrelated call paths, and more than one ownership design could fit.\n\nFix:\nChoose the design explicitly: actor ownership, one owner plus borrowed views, or reviewed shared mutation.",
        ),
        crate::KErrorCode::K0100 => Some(
            "Problem:\nlet msg = queue.recv();\nreturn Ok(());\n\nFix:\nCall ack, nack, requeue, commit, rollback, or record explicit debt before the obligation leaves the checked scenario.",
        ),
        crate::KErrorCode::K0102 => Some(
            "Problem:\nlet id = random_uuid();\nwrite_event(id);\n\nFix:\nRoute the value through a deterministic facade, record the value in the event stream, or mark replay debt for this path.",
        ),
        crate::KErrorCode::K0107 => Some(
            "Problem:\nlet response = reqwest::get(url).await?;\n\nFix:\nDeclare whether this boundary is modeled, recorded, outside replay, opaque, or accepted as debt.",
        ),
        crate::KErrorCode::K0108 => Some(
            "Problem:\n#[kobo::suppress(replay)]\ncall_external_service();\n\nFix:\nAttach a specific reason and owner to the suppression, then replace it with a model or boundary policy when the path becomes covered.",
        ),
        _ => None,
    }
}

fn reserved_fix_guidance() -> &'static str {
    "- If you saw this code in normal compiler output, report a Kobo bug with the command and source file.\n\
     - If you are reading the catalog, treat this as a future diagnostic slot.\n\
     - Do not write tests or suppressions that depend on this slot until it becomes active."
}

fn reserved_example_guidance() -> &'static str {
    "Problem:\nA build prints K0003 even though this Kobo version marks that slot as reserved.\n\n\
     Fix:\nReport the compiler output as a Kobo bug, because reserved slots should not be emitted as real user diagnostics."
}

fn explain_prose(code: crate::KErrorCode) -> Option<ExplainProse> {
    ownership_explain_prose(code)
        .or_else(|| performance_explain_prose(code))
        .or_else(|| strict_boundary_explain_prose(code))
        .or_else(|| async_explain_prose(code))
        .or_else(|| design_explain_prose(code))
        .or_else(|| migration_explain_prose(code))
        .or_else(|| scenario_explain_prose(code))
        .or_else(|| project_prose::ecosystem_explain_prose(code))
        .or_else(|| project_prose::parser_explain_prose(code))
}

fn ownership_explain_prose(code: crate::KErrorCode) -> Option<ExplainProse> {
    let prose = match code {
        crate::KErrorCode::K0001 => ExplainProse {
            fix: "Want to use this value again after sending it somewhere?\n\
                  - Borrow it if the other function only needs to read it.\n\
                  - Clone only the small part you still need.\n\
                  - Move the later use before the call that takes the value.",
            example: "Problem:\nlet user = load_user();\nsend(user);\nlog(user.name);\n\n\
                      Fix:\nBorrow for the send call, log before the move, or clone only the field that must remain available.",
        },
        crate::KErrorCode::K0002 => ExplainProse {
            fix: "Want to change this value while another borrow is still live?\n\
                  - End the earlier borrow before starting the mutation.\n\
                  - Split the mutation into a later scope.\n\
                  - Borrow a smaller field when only part of the value needs mutation.",
            example: "Problem:\nlet name = &profile.name;\nprofile.name.push_str(\"!\");\nprintln(name);\n\n\
                      Fix:\nEnd the read before the mutation, or move the mutation into a later block after the read is done.",
        },
        crate::KErrorCode::K0019 => ExplainProse {
            fix: "- Split the large function so Kobo can report each borrow region precisely.\n\
                  - Move nested borrow-heavy work into a helper.\n\
                  - Rerun after reducing the number of live borrow records in one generated function.",
            example: "Problem:\nOne generated function creates more borrow records than Kobo can report precisely.\n\n\
                      Fix:\nMove part of the work into a helper so each function has a clear borrow count.",
        },
        _ => return None,
    };

    Some(prose)
}

fn performance_explain_prose(code: crate::KErrorCode) -> Option<ExplainProse> {
    let prose = match code {
        crate::KErrorCode::K0020 => ExplainProse {
            fix: "- Reuse storage outside the hot loop when the value does not need fresh allocation.\n\
                  - Keep the allocation and record why the cost is acceptable.\n\
                  - Move the hot path into a helper so the allocation site is obvious.",
            example: "Problem:\nA hot loop allocates a fresh buffer every iteration.\n\n\
                      Fix:\nReuse the buffer outside the loop or keep the current code with a documented reason.",
        },
        crate::KErrorCode::K0021 => ExplainProse {
            fix: "- Pass the value by borrow when the callee only reads it.\n\
                  - Keep the clone when later mutation needs independent storage.\n\
                  - Add a local name before the clone so reviewers can see why it is needed.",
            example: "Problem:\nworker(payload.clone()) runs even though worker only reads payload.\n\n\
                      Fix:\nPass &payload, or keep the clone only when the worker must own it.",
        },
        crate::KErrorCode::K0026 => ExplainProse {
            fix: "- Remove the attribute if it no longer changes checking behavior.\n\
                  - Move the attribute to the item it was meant to affect.\n\
                  - Fix the attribute spelling or arguments before relying on it.",
            example: "Problem:\nA relax attribute appears on a statement that Kobo does not relax.\n\n\
                      Fix:\nRemove it, or move a correctly spelled attribute to the item that needs it.",
        },
        crate::KErrorCode::K0031 => ExplainProse {
            fix: "- Keep the value plain-owned when Kobo cannot represent the requested owner safely.\n\
                  - Add a smaller wrapper type that makes the intended owner clear.\n\
                  - Split the binding so only the safe part uses the stronger ownership shape.",
            example: "Problem:\nA generated binding asks for an ownership shape Kobo cannot preserve here.\n\n\
                      Fix:\nKeep this binding plain-owned or split it into smaller values with clear owners.",
        },
        crate::KErrorCode::K0032 => ExplainProse {
            fix: "- End the borrow before moving the value.\n\
                  - Move only a field that is not borrowed.\n\
                  - Use shared ownership only when the value must stay available in both places.",
            example: "Problem:\nA value is moved while a borrow of that value is still live.\n\n\
                      Fix:\nEnd the borrow first, or choose shared ownership intentionally.",
        },
        _ => return None,
    };

    Some(prose)
}

fn strict_boundary_explain_prose(code: crate::KErrorCode) -> Option<ExplainProse> {
    let prose = match code {
        crate::KErrorCode::K0025 => ExplainProse {
            fix: "- Remove the hint when the requested ownership shape is not important.\n\
                  - Change the code so the value has only one owner.\n\
                  - Move the hinted code outside the strict region when it is only a performance choice.",
            example: "Problem:\n#[kobo::hint(ownership = \"move\")]\nlet buf = String::new();\nuse_mut(&mut buf);\nuse_mut(&mut buf);\n\n\
                      Fix:\nRemove the hint, or refactor so buf has only one owner.",
        },
        crate::KErrorCode::K0030 => ExplainProse {
            fix: "- Keep one owner for the resource handle.\n\
                  - Borrow the handle instead of moving it when later code still needs it.\n\
                  - Open a separate handle only when two independent handles are intended.",
            example: "Problem:\nA file handle is moved into one helper and then used again by another helper.\n\n\
                      Fix:\nBorrow the handle, reorder the use, or create a second handle intentionally.",
        },
        crate::KErrorCode::K0041 => ExplainProse {
            fix: "Want this strict block to own the value clearly?\n\
                  - Finish using or drop the aliases before entering the @strict block.\n\
                  - Move the strict update into a helper that owns the value.\n\
                  - Keep this code outside strict when an alias must stay live.",
            example: "Problem:\nlet alias = cache.view();\n@strict { cache.update(); }\nuse(alias);\n\n\
                      Fix:\nFinish using or drop the alias before entering the @strict block.",
        },
        crate::KErrorCode::K0042 => ExplainProse {
            fix: "- Move the closure outside the @strict block.\n\
                  - Pass plain data into the closure so it does not capture the guarded value.\n\
                  - Run the closure inside the strict block before it can escape.",
            example: "Problem:\n@strict {\n    let bump = || cache.update();\n    save(bump);\n}\n\n\
                      Fix:\nMove the closure outside the strict block, or pass plain data into the closure instead of capturing the guarded value.",
        },
        crate::KErrorCode::K0043 => ExplainProse {
            fix: "- Do not move the guarded value out of the @strict block.\n\
                  - Return a reviewed result instead of the guarded value itself.\n\
                  - Split the strict work so ownership is restored before leaving the block.",
            example: "Problem:\nA value protected by @strict is moved into another owner inside the block.\n\n\
                      Fix:\nKeep the guarded value in place and return only the computed result.",
        },
        crate::KErrorCode::K0044 => ExplainProse {
            fix: "- Replace the labeled break or continue with a local return value.\n\
                  - Move the loop outside the @strict block.\n\
                  - End strict work before jumping to an outer label.",
            example: "Problem:\nA labeled continue jumps out of @strict before Kobo can close the guarded region.\n\n\
                      Fix:\nUse a local flag or move the loop boundary outside @strict.",
        },
        _ => return None,
    };

    Some(prose)
}

fn async_explain_prose(code: crate::KErrorCode) -> Option<ExplainProse> {
    let prose = match code {
        crate::KErrorCode::K0060 => ExplainProse {
            fix: "- End the RefCell borrow before the next .await.\n\
                  - Move the borrowed work into a small non-async helper.\n\
                  - Store owned data before awaiting instead of keeping the borrow live.",
            example: "Problem:\nlet guard = state.borrow_mut();\nwait().await;\ndrop(guard);\n\n\
                      Fix:\nPut the borrow in a block that ends before wait().await.",
        },
        crate::KErrorCode::K0061 => ExplainProse {
            fix: "- Move only thread-safe data into the future.\n\
                  - Use LocalSet when this task is intentionally single-thread local.\n\
                  - Use #[kobo::async_shared] when shared async ownership is intentional.\n\
                  - Keep the value on the current task and await a local helper.",
            example: "Problem:\nA future passed to a multi-thread executor captures a value that must stay on one thread.\n\n\
                      Before:\nlet guard = &mut shared;\ntokio::spawn(async move {\n    guard.push(1);\n});\n\n\
                      Fix:\nRun it on LocalSet when the task is local:\nlet local = tokio::task::LocalSet::new();\nlocal.spawn_local(async move {\n    guard.push(1);\n});\n\n\
                      Or capture only thread-safe owned data before spawning.\n\
                      Or mark intentional shared async ownership with #[kobo::async_shared].",
        },
        crate::KErrorCode::K0062 => ExplainProse {
            fix: "- Add tokio or async-std to [dependencies].\n\
                  - Keep this code synchronous until runtime setup is explicit.\n\
                  - Move async code behind a caller that already owns the runtime.",
            example: "Problem:\nasync fn main() {\n    handle().await;\n}\n\n\
                      Fix:\nAdd a supported executor dependency such as tokio or async-std, or keep the entry point synchronous until runtime setup is explicit.",
        },
        crate::KErrorCode::K0063 => ExplainProse {
            fix: "- Use `@strict async fn` if the whole function should follow Kobo's async strict rules.\n\
                  - Move this strict work into a small non-async helper.\n\
                  - End the strict borrow before the next .await.",
            example: "Problem:\nasync fn handler() {\n    @strict { data.borrow_mut().push(1); }\n    wait().await;\n}\n\n\
                      Fix:\nUse `@strict async fn` if the whole function should follow Kobo's async strict rules, or move this strict work into a small non-async helper.",
        },
        crate::KErrorCode::K0064 => ExplainProse {
            fix: "- Move @strict work into a non-async helper.\n\
                  - Finish the strict block before any .await can happen.\n\
                  - Mark the whole async function strict only when every pause point follows the strict async rules.",
            example: "Problem:\nAn @strict block appears inside an async block that can pause.\n\n\
                      Fix:\nMove the strict mutation into a synchronous helper and call it before .await.",
        },
        crate::KErrorCode::K0065 => ExplainProse {
            fix: "- Make every select branch complete or compensate the active value.\n\
                  - Move unfinished values outside the cancellable branch.\n\
                  - Split the work so cancellation can only happen after cleanup.",
            example: "Problem:\nOne select branch returns while another branch owns an unfinished delivery.\n\n\
                      Fix:\nAck, nack, requeue, or move the delivery out before select can cancel the branch.",
        },
        crate::KErrorCode::K0067 => ExplainProse {
            fix: "- Keep request-scoped state inside the request task.\n\
                  - Move only owned data across the async boundary.\n\
                  - Add an explicit cleanup path before the state can be dropped.",
            example: "Problem:\nA handler stores request state in a spawned task that can outlive the request.\n\n\
                      Fix:\nClone only safe owned data into the task or finish the state before spawning.",
        },
        _ => return None,
    };

    Some(prose)
}

fn design_explain_prose(code: crate::KErrorCode) -> Option<ExplainProse> {
    let prose = match code {
        crate::KErrorCode::K0080 => ExplainProse {
            fix: "- Choose an explicit ownership design before migration.\n\
                  - Record reviewed debt when the architecture decision must wait.\n\
                  - Split the value so each part has one clear owner.",
            example: "Problem:\nState is mutated from several unrelated call paths, and more than one ownership design could fit.\n\n\
                      Fix:\nChoose the design explicitly: actor ownership, one owner plus borrowed views, or reviewed shared mutation.",
        },
        crate::KErrorCode::K0080P1 => ExplainProse {
            fix: "- Pick the migration ownership pattern before changing code.\n\
                  - Wrap the value in an owner type when one component should control it.\n\
                  - Record debt if the architecture decision must be made later.",
            example: "Problem:\nA migration path could use actor ownership or shared ownership, and Kobo cannot choose for the team.\n\n\
                      Fix:\nPick the intended ownership pattern and make it explicit in the migrated code.",
        },
        crate::KErrorCode::K0080P2 => ExplainProse {
            fix: "- Replace the strong back-pointer with Weak.\n\
                  - Store parent ids instead of parent pointers.\n\
                  - Move ownership to a tree owner that manages parent and child links.",
            example: "Problem:\nA child node holds a strong pointer back to its parent, creating a cycle risk.\n\n\
                      Fix:\nUse Weak for the back-pointer or store a parent id.",
        },
        crate::KErrorCode::K0080P3 => ExplainProse {
            fix: "- Introduce a single owner that performs the mutations.\n\
                  - Pass borrowed views to read-only call sites.\n\
                  - Keep shared mutation only with an explicit reviewed policy.",
            example: "Problem:\nThree call sites mutate the same state through shared handles.\n\n\
                      Fix:\nMove mutation behind one owner or record the shared-mutation policy intentionally.",
        },
        crate::KErrorCode::K0080P4 => ExplainProse {
            fix: "- Store recursive data behind Box, Arc, or another pointer type.\n\
                  - Remove the direct self-reference if the type should be finite.\n\
                  - Split parent and child data into separate owned values.",
            example: "Problem:\nstruct Node { child: Node } has no finite size.\n\n\
                      Fix:\nUse Box<Node> or another indirection for the child.",
        },
        crate::KErrorCode::K0081 => ExplainProse {
            fix: "- Split the large ownership problem into smaller helper functions.\n\
                  - Add a local ownership annotation at the value Kobo highlights.\n\
                  - Name intermediate results so each transfer has one obvious owner.",
            example: "Problem:\nOne expression combines many ownership transfers at once.\n\n\
                      Fix:\nBreak it into named steps or add the missing ownership annotation.",
        },
        crate::KErrorCode::K0082 => ExplainProse {
            fix: "- Simplify the highlighted expression into smaller statements.\n\
                  - Add the missing ownership annotation instead of asking Kobo to infer it.\n\
                  - Split this function if many unrelated ownership decisions happen together.",
            example: "Problem:\nOne function has too many ownership choices for Kobo to finish quickly.\n\n\
                      Fix:\nAdd the key annotation or move part of the work into a helper.",
        },
        crate::KErrorCode::K0083 => ExplainProse {
            fix: "- Pick the ownership form you intended at the highlighted value.\n\
                  - Add a local annotation so future edits keep the same meaning.\n\
                  - Split the branch if two different meanings are being mixed.",
            example: "Problem:\nBoth borrow and move would type-check, but they mean different ownership behavior.\n\n\
                      Fix:\nWrite the ownership choice explicitly.",
        },
        crate::KErrorCode::K0084 => ExplainProse {
            fix: "- Accept the suggested ownership choice only after checking the generated Rust.\n\
                  - Add an explicit annotation if the suggestion is correct.\n\
                  - Rewrite the expression if the suggested owner is not the one you intended.",
            example: "Problem:\nKobo picked a likely owner, but the source does not make that choice clear.\n\n\
                      Fix:\nConfirm the generated Rust and then make the owner explicit.",
        },
        crate::KErrorCode::K0085 => ExplainProse {
            fix: "- Do not rely on the suggested rewrite until you review it.\n\
                  - Write the ownership annotation yourself.\n\
                  - Split the code so Kobo no longer has to guess.",
            example: "Problem:\nKobo found a possible ownership rewrite with weak confidence.\n\n\
                      Fix:\nMake the transfer explicit or rewrite the code into smaller steps.",
        },
        _ => return None,
    };

    Some(prose)
}

fn migration_explain_prose(code: crate::KErrorCode) -> Option<ExplainProse> {
    let prose = match code {
        crate::KErrorCode::K0090 => ExplainProse {
            fix: "- Wrap the external crate call in a Kobo boundary declaration.\n\
                  - Move migration to the caller that owns the external value.\n\
                  - Record reviewed debt when the external crate must stay unchanged.",
            example: "Problem:\nA migrated value crosses into code Kobo cannot rewrite.\n\n\
                      Fix:\nDeclare the boundary or keep the migration debt visible for review.",
        },
        crate::KErrorCode::K0095 => ExplainProse {
            fix: "- Add an explicit ownership annotation around the macro output.\n\
                  - Wrap macro-generated values in a small named helper.\n\
                  - Keep this migration as reviewed debt until the macro is modeled.",
            example: "Problem:\nA macro creates a value whose owner is not visible in the Kobo source.\n\n\
                      Fix:\nWrap the macro call and state who owns the generated value.",
        },
        crate::KErrorCode::K0096 => ExplainProse {
            fix: "- Move the setting to `kobo check --profile dev|checked|release`.\n\
                  - Record the guarantee policy in Kobo.toml.\n\
                  - Replace the legacy directive with `//! kobo:profile = \"release\"` while the source-level profile migration is active.",
            example: "Problem:\n//! kobo:mode = strict\n\n\
                      Fix:\nUse `--profile release` or project guarantee policy so the file remains ordinary Kobo source.",
        },
        crate::KErrorCode::K0099 => ExplainProse {
            fix: "- Fix the highlighted Kobo source that produced the Rust error.\n\
                  - Regenerate Rust and source maps together if the span looks stale.\n\
                  - Check the original Rust error only when the Kobo span is missing.",
            example: "Problem:\nRust reports an error in generated code, and Kobo maps it back to the original source.\n\n\
                      Fix:\nEdit the mapped Kobo source, then regenerate the Rust output.",
        },
        _ => return None,
    };

    Some(prose)
}

fn scenario_explain_prose(code: crate::KErrorCode) -> Option<ExplainProse> {
    let prose = match code {
        crate::KErrorCode::K0100 => ExplainProse {
            fix: "Want to finish the delivery here?\n\
                  - ack(delivery): the message was handled.\n\
                  - nack(delivery): the message failed.\n\
                  - requeue(delivery): try the message again later.\n\
                  - debt(delivery): cleanup happens somewhere else and you want that visible.",
            example: "Problem:\nlet msg = queue.recv();\nreturn Ok(());\n\n\
                      Fix:\nCall ack, nack, requeue, commit, rollback, or debt on every path before the delivery leaves this scenario.",
        },
        crate::KErrorCode::K0101 => ExplainProse {
            fix: "- Complete the obligation before it leaves the checked function.\n\
                  - Return a wrapper that carries the obligation policy explicitly.\n\
                  - Move the obligation boundary to a caller Kobo can check.",
            example: "Problem:\nA delivery obligation is stored in a field and leaves local analysis without ack, nack, or requeue.\n\n\
                      Fix:\nFinish the delivery locally or return a wrapper with an explicit policy.",
        },
        crate::KErrorCode::K0102 => ExplainProse {
            fix: "Want this path to replay the same way later?\n\
                  - Route values that can change through a deterministic facade.\n\
                  - Record the value in the event stream.\n\
                  - Move the changing value outside replay or mark replay debt.",
            example: "Problem:\nlet id = random_uuid();\nwrite_event(id);\n\n\
                      Fix:\nRoute the value through a deterministic facade, record the value in the event stream, or mark replay debt for this path.",
        },
        crate::KErrorCode::K0103 => ExplainProse {
            fix: "Want Kobo to replay this action later?\n\
                  - Record the external effect before replaying the scenario.\n\
                  - Replace the effect with a deterministic model in scenario mode.\n\
                  - Mark the path outside replay when the action cannot be modeled.",
            example: "Problem:\nA scenario reads the current time during replay.\n\n\
                      Fix:\nRecord the time value, model it, or keep that call outside replay.",
        },
        crate::KErrorCode::K0104 => ExplainProse {
            fix: "- Compare the recorded witness with the current scenario inputs.\n\
                  - Regenerate the witness after intentional behavior changes.\n\
                  - Keep the old witness when the behavior change was not intended and fix the code.",
            example: "Problem:\nThe replayed scenario takes a different branch than the recorded witness.\n\n\
                      Fix:\nRegenerate the witness only if the new behavior is expected.",
        },
        crate::KErrorCode::K0105 => ExplainProse {
            fix: "- Reduce the scenario size for the quick profile.\n\
                  - Move this scenario to a slower profile with an explicit budget.\n\
                  - Split one large scenario into focused smaller scenarios.",
            example: "Problem:\nA quick scenario runs longer than the allowed budget.\n\n\
                      Fix:\nShrink the scenario or move it to a profile that allows the longer run.",
        },
        crate::KErrorCode::K0106 => ExplainProse {
            fix: "- Keep the larger witness when the smaller one changes the failure.\n\
                  - Add a stronger replay check before accepting the shrink.\n\
                  - Regenerate the witness after fixing the scenario model.",
            example: "Problem:\nA minimized witness no longer reproduces the same failure.\n\n\
                      Fix:\nReject that shrink and keep the smallest witness that still fails for the same reason.",
        },
        crate::KErrorCode::K0107 => ExplainProse {
            fix: "- Choose model, record, or outside when this path should stay replayable.\n\
                  - Choose opaque or debt when the boundary is accepted but not fully modeled.\n\
                  - Wrap the external call so the policy is visible at one call site.",
            example: "Problem:\nlet response = reqwest::get(url).await?;\n\n\
                      Fix:\nDeclare whether this boundary is modeled, recorded, outside replay, opaque, or accepted as debt.",
        },
        crate::KErrorCode::K0108 => ExplainProse {
            fix: "- Keep the suppression reason specific and reviewable.\n\
                  - Replace the suppression with a model or boundary policy when the path becomes covered.\n\
                  - Assign an owner to remove the suppression later.",
            example: "Problem:\n#[kobo::suppress(replay)]\ncall_external_service();\n\n\
                      Fix:\nAttach a specific reason and owner to the suppression, then replace it with a model or boundary policy when the path becomes covered.",
        },
        crate::KErrorCode::K0109 => ExplainProse {
            fix: "- Use only fields allowed by the capability view.\n\
                  - Add a narrower helper that exposes the field intentionally.\n\
                  - Change the capability declaration if the field should be available.",
            example: "Problem:\nA read-only view is used to mutate a hidden field.\n\n\
                      Fix:\nUse an allowed field or change the capability view intentionally.",
        },
        crate::KErrorCode::K0114 => ExplainProse {
            fix: "- Write the required must_call target in the attribute.\n\
                  - Remove the attribute until the required call is known.\n\
                  - Keep the target name simple so Kobo can match it on every path.",
            example: "Problem:\n#[must_call] does not say which method must be called.\n\n\
                      Fix:\nUse an attribute such as #[must_call(\"close\")] or remove the incomplete attribute.",
        },
        crate::KErrorCode::K0115 => ExplainProse {
            fix: "- Regenerate the witness with the current Kobo version.\n\
                  - Fix the witness schema before relying on replay results.\n\
                  - Keep the old witness only as an artifact, not as checked evidence.",
            example: "Problem:\nA .kwit file is missing a required scenario field.\n\n\
                      Fix:\nRegenerate the witness or update it to the current schema.",
        },
        crate::KErrorCode::K0116 => ExplainProse {
            fix: "- Split the scenario around the unsupported construct.\n\
                  - Keep the witness partial until that construct has a model.\n\
                  - Replace the construct with a modeled Kobo facade for exact replay.",
            example: "Problem:\nA scenario uses tokio::select!, but this Kobo version cannot model every cancellation branch for exact replay.\n\n\
                      Fix:\nKeep the witness partial or rewrite the scenario through a modeled cancellation-safe facade.",
        },
        crate::KErrorCode::K0117 => ExplainProse {
            fix: "- Regenerate the witness with --engine both.\n\
                  - Compare semantic_trace_hash and harness_trace_hash before trusting replay.\n\
                  - Treat the witness as invalid until the mismatch is explained.",
            example: "Problem:\nThe compiler semantic trace says an ack happened, but the generated harness trace does not.\n\n\
                      Fix:\nRegenerate the witness and investigate the trace mismatch before claiming exact replay.",
        },
        crate::KErrorCode::K0118 => ExplainProse {
            fix: "- Regenerate the witness or session artifact for the current source.\n\
                  - Ignore stale artifacts whose source hash or target does not match.\n\
                  - Rerun the command that produced the artifact before using editor actions.",
            example: "Problem:\nAn LSP replay action points at a witness from an older source hash.\n\n\
                      Fix:\nRerun kobo test --sim quick --engine both and use the new witness path.",
        },
        _ => return None,
    };

    Some(prose)
}
