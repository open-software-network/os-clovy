use super::*;

#[test]
fn launch_failure_distinguishes_typed_denial_from_a_lookalike_error() {
    let denial = crate::computer_use_policy::PrivateBrowserLaunchDenied;
    let known_error = anyhow::Error::new(crate::computer_use_policy::PrivateBrowserLaunchDenied);
    let known = launch_failure(known_error.as_ref(), "visible");
    assert_eq!(known["isError"], true);
    assert_eq!(
        known["structuredContent"]["code"],
        "computer_use_private_browser_blocked"
    );
    assert_eq!(
        known["structuredContent"]["disposition"]["delivery"],
        "not_delivered"
    );
    assert_eq!(known["structuredContent"]["disposition"]["retry"], "safe");
    let lookalike = std::io::Error::other(denial.to_string());
    let unknown = launch_failure(&lookalike, "visible");
    assert_eq!(unknown["isError"], true);
    assert_eq!(
        unknown["structuredContent"]["code"],
        "computer_use_launch_unverified"
    );
    assert_eq!(
        unknown["structuredContent"]["disposition"]["delivery"],
        "delivery_uncertain"
    );
    assert_eq!(
        unknown["structuredContent"]["disposition"]["retry"],
        "unsafe"
    );
}
fn window() -> Window {
    Window {
        public_id: 7,
        raw: json!({"id":"w-7","pid":42,"process_instance":"p42:100","native_owner":{"pid":77,"instance":"p77:200"},"app_name":"Synthetic","title":"Document","is_focused":false,"visible":true,"bounds":{"x":-200.0,"y":10.0,"width":400.0,"height":300.0}}),
    }
}
fn capture() -> Capture {
    let mut elements = HashMap::new();
    let mut text = String::new();
    flatten_tree(
        &json!({"role":"window","children":[{"ref_id":"@snapshot:e4","role":"textfield","name":"Body","bounds":{"x":-180.0,"y":30.0,"width":200.0,"height":100.0},"available_actions":["TypeText","SetValue"]}]}),
        0,
        &mut elements,
        &mut text,
    );
    Capture {
        window: window(),
        elements,
        width: 800,
        height: 600,
        mode: "som".into(),
        complete: true,
    }
}
#[test]
fn exact_window_process_instance_and_geometry_are_required() {
    let w = window();
    let mut changed = w.clone();
    changed.raw["process_instance"] = json!("p42:101");
    assert!(!w.same_identity(&changed));
    changed = w.clone();
    changed.raw["native_owner"]["instance"] = json!("p77:201");
    assert!(!w.same_identity(&changed));
    changed = w.clone();
    changed.raw["bounds"]["x"] = json!(-201.0);
    assert!(w.same_identity(&changed));
    assert!(!w.same_capture(&changed));
    changed = w.clone();
    changed.raw["title"] = json!("Other document");
    assert!(!w.same_capture(&changed));
    changed = w.clone();
    changed.raw["process_instance"] = Value::Null;
    assert!(!changed.same_identity(&changed));
}
#[test]
fn image_local_coordinates_scale_with_exact_capture_and_negative_origin() {
    let c = capture();
    assert_eq!(
        local_point(&json!({"x":200,"y":100}), "x", "y", &c).unwrap(),
        (-100.0, 60.0)
    );
    assert!(local_point(&json!({"x":800,"y":100}), "x", "y", &c).is_err());
    assert!(local_point(&json!({"x":-1,"y":100}), "x", "y", &c).is_err());
}
#[test]
fn insert_and_replace_remain_distinct_and_argv_preserves_literal_unicode() {
    let c = capture();
    let e = c.elements.get(&1).unwrap();
    let hostile = "--help\n$(touch /tmp/never) `echo nope` Olá 🦀";
    let typed = action_argv("type_text", &json!({"text":hostile}), &c, Some(e), false).unwrap();
    assert_eq!(typed[0], "type");
    assert_eq!(typed[1], "--timeout-ms");
    assert_eq!(typed[3], "@snapshot:e4");
    assert_eq!(typed[4], "--");
    assert_eq!(typed[5], hostile);
    for literal in ["--session", "--help", "--headed", ""] {
        let args = action_argv("type_text", &json!({"text":literal}), &c, Some(e), true).unwrap();
        assert_eq!(args[0], "--headed");
        assert_eq!(
            &args[args.len() - 2..],
            &["--".to_string(), literal.to_string()]
        );
    }
    assert!(!typed.contains(&"set-value".into()));
    let replaced = action_argv(
        "set_value",
        &json!({"value":"replacement"}),
        &c,
        Some(e),
        false,
    )
    .unwrap();
    assert_eq!(replaced[0], "set-value");
}
#[test]
fn click_double_right_drag_scroll_and_keys_retain_scope() {
    let c = capture();
    let e = c.elements.get(&1).unwrap();
    for (tool, command) in [
        ("click", "click"),
        ("double_click", "double-click"),
        ("right_click", "right-click"),
    ] {
        let argv = action_argv(tool, &json!({}), &c, Some(e), tool == "double_click").unwrap();
        assert!(argv.contains(&command.into()));
        assert!(argv.contains(&e.reference));
    }
    let click = action_argv("click", &json!({"x":200,"y":100}), &c, None, true).unwrap();
    assert!(click.contains(&"--xy=-100,60".into()));
    assert!(click.contains(&"--clovy-target-json".into()));
    let double = action_argv("click", &json!({"x":200,"y":100,"count":2}), &c, None, true).unwrap();
    assert_eq!(
        double[double.iter().position(|a| a == "--count").unwrap() + 1],
        "2"
    );
    let modified_drag = action_argv(
        "drag",
        &json!({"from_x":200,"from_y":100,"to_x":600,"to_y":500,"modifier":["shift","alt"]}),
        &c,
        None,
        true,
    )
    .unwrap();
    assert_eq!(
        modified_drag
            .iter()
            .filter(|a| a.as_str() == "--clovy-modifier")
            .count(),
        2
    );
    let drag = action_argv(
        "drag",
        &json!({"from_x":200,"from_y":100,"to_x":600,"to_y":500}),
        &c,
        None,
        true,
    )
    .unwrap();
    assert_eq!(drag[0], "--headed");
    assert!(drag.contains(&"--clovy-target-json".into()));
    let scroll = action_argv(
        "scroll",
        &json!({"direction":"down","amount":3}),
        &c,
        Some(e),
        false,
    )
    .unwrap();
    assert_eq!(scroll[0], "scroll");
    assert!(scroll.contains(&e.reference));
    let key = action_argv("press_key", &json!({"key":"escape"}), &c, None, false).unwrap();
    assert_eq!(&key[..4], ["press", "escape", "--window-id", "w-7"]);
    let at = key
        .iter()
        .position(|argument| argument == "--clovy-target-json")
        .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&key[at + 1]).unwrap(),
        c.window.raw
    );
    let hotkey = action_argv("hotkey", &json!({"keys":["ctrl","f"]}), &c, None, false).unwrap();
    assert_eq!(hotkey[1], "ctrl+f");
    assert!(hotkey.contains(&"w-7".into()));
}
#[test]
fn metadata_detects_hidden_password_and_otp_without_value_replacement() {
    assert!(sensitive(
        &json!({"role":"textfield","states":["secure"],"name":""})
    ));
    assert!(sensitive(
        &json!({"role":"textfield","subrole":"AXSecureTextField","name":"Field"})
    ));
    assert!(sensitive(
        &json!({"role":"textfield","native_id":{"value":"one-time-code"}})
    ));
    assert!(sensitive(
        &json!({"role":"textfield","placeholder":"Card number"})
    ));
    assert!(!sensitive(&json!({"role":"textfield","name":"Body"})));
    assert!(!editable(&json!({"role":"button","name":"Body"})));
}
#[test]
fn only_unsupported_definitively_not_delivered_can_escalate() {
    let unsupported = json!({"version":"2.5","ok":false,"error":{"code":"ACTION_NOT_SUPPORTED","disposition":{"delivery":"not_delivered","retry":"safe"}}});
    assert!(may_escalate(&unsupported));
    for delivery in [
        "unknown",
        "delivery_uncertain",
        "delivered_unverified",
        "delivered_verified",
    ] {
        let mut result = unsupported.clone();
        result["error"]["disposition"]["delivery"] = json!(delivery);
        assert!(!may_escalate(&result));
    }
    let mut result = unsupported.clone();
    result["error"]["code"] = json!("POLICY_DENIED");
    assert!(!may_escalate(&result));
}
#[test]
fn receipts_keep_engine_evidence_and_uncertainty_without_flattening() {
    let engine = json!({"version":"2.5","ok":false,"error":{"code":"TIMEOUT","message":"Target stopped responding","disposition":{"delivery":"delivery_uncertain","retry":"unsafe"},"details":{"steps":[{"mechanism":"physical_synthetic","outcome":"succeeded"}]}}});
    let result = normalize(engine.clone(), "visible");
    assert_eq!(result["structuredContent"]["engine_evidence"], engine);
    assert_eq!(
        result["structuredContent"]["disposition"]["retry"],
        "unsafe"
    );
    assert_eq!(result["structuredContent"]["effective_modality"], "visible");
    assert_eq!(result["isError"], true);
    let transport = failure("computer_use_engine_transport", "Lost reply", true);
    assert_eq!(
        transport["structuredContent"]["disposition"]["delivery"],
        "delivery_uncertain"
    );
}
#[test]
fn som_marks_use_scaled_capture_geometry_and_numbered_ref_identity() {
    use image::ImageEncoder;
    let c = capture();
    let mut png = Vec::new();
    let white = vec![255u8; 800 * 600 * 4];
    image::codecs::png::PngEncoder::new(&mut png)
        .write_image(&white, 800, 600, image::ExtendedColorType::Rgba8)
        .unwrap();
    let annotated = draw_marks(&png, &c.window, &c.elements, c.width, c.height).unwrap();
    let rendered = image::load_from_memory(&annotated).unwrap().to_rgba8();
    assert_eq!(rendered.get_pixel(40, 40).0, [27, 85, 220, 255]);
    assert_eq!(rendered.get_pixel(39, 39).0, [255, 255, 255, 255]);
}

#[test]
fn value_updates_keep_nontext_controls_and_unicode_character_limit() {
    let slider = Element {
        reference: "@capture:e1".into(),
        node: json!({"role":"slider","available_actions":["SetValue"]}),
        native: Value::Null,
    };
    assert!(validate_value_action("set_value", &json!({"value":"50"}), &slider).is_ok());
    assert!(validate_value_action("type_text", &json!({"text":"50"}), &slider).is_err());
    let field = Element {
        reference: "@capture:e2".into(),
        node: json!({"role":"textfield"}),
        native: Value::Null,
    };
    assert!(
        validate_value_action("type_text", &json!({"text":"🦀".repeat(10_000)}), &field).is_ok()
    );
    assert!(
        validate_value_action("type_text", &json!({"text":"🦀".repeat(10_001)}), &field).is_err()
    );
}

#[test]
fn explicit_capture_never_reuses_public_refs_and_private_revalidation_preserves_authority() {
    let original = capture();
    let mut next = 1;
    let first = bind_public_elements(original.elements.clone(), None, false, &mut next).unwrap();
    let first_id = *first.keys().next().unwrap();
    let mut baseline = original.clone();
    baseline.elements = first;
    let mut inspected = original.elements.clone();
    inspected.get_mut(&1).unwrap().reference = "@fresh:e1".into();
    let checked =
        bind_public_elements(inspected.clone(), Some(&baseline), true, &mut next).unwrap();
    assert_eq!(
        render_elements(&baseline.elements),
        render_elements(&checked)
    );
    assert_eq!(
        checked[&first_id].reference,
        baseline.elements[&first_id].reference
    );
    assert!(!render_elements(&checked).contains("ref_id"));
    assert!(!render_elements(&checked).contains("@fresh"));
    let second = bind_public_elements(inspected.clone(), None, false, &mut next).unwrap();
    assert!(!second.contains_key(&first_id));
    inspected.get_mut(&1).unwrap().node["states"] = json!(["secure"]);
    assert!(bind_public_elements(inspected, Some(&baseline), true, &mut next).is_err());
    let mut replaced = original.elements.clone();
    replaced.get_mut(&1).unwrap().native = json!({"retained_object":"other-native-control"});
    assert!(bind_public_elements(replaced, Some(&baseline), true, &mut next).is_err());
}

#[test]
fn content_only_renderers_keep_effective_modality_and_original_error_cause() {
    let failed = normalize(
        json!({"ok":false,"error":{"code":"NATIVE_FAILURE","message":"Exact native cause.","disposition":{"delivery":"delivery_uncertain","retry":"unsafe"}}}),
        "visible",
    );
    assert_eq!(
        failed["content"][0]["text"],
        "Exact native cause. The effective mode was visible."
    );
    let completed = normalize(json!({"ok":true,"data":{}}), "background");
    assert!(completed["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("background mode"));
    let observed = success(json!({}), vec![], "background");
    assert!(observed["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("background mode"));
}

#[test]
fn definitive_native_capabilities_choose_visible_before_dispatch_but_partial_evidence_does_not() {
    let mut c = capture();
    let mut e = c.elements[&1].clone();
    e.native = json!({"available_actions":["SetValue","SetFocus"]});
    e.node["available_actions"] = e.native["available_actions"].clone();
    assert!(semantic_capability_absent(&c, Some(&e), "type_text"));
    assert!(semantic_capability_absent(&c, Some(&e), "right_click"));
    c.complete = false;
    assert!(!semantic_capability_absent(&c, Some(&e), "type_text"));
    c.complete = true;
    e.node["available_actions"] = Value::Null;
    assert!(!semantic_capability_absent(&c, Some(&e), "right_click"));
    e.node["available_actions"] = json!(["SetValue"]);
    assert!(!semantic_capability_absent(&c, Some(&e), "right_click"));
    e.node["available_actions"] = json!(["TypeText", "RightClick"]);
    e.native["available_actions"] = e.node["available_actions"].clone();
    assert!(!semantic_capability_absent(&c, Some(&e), "type_text"));
    assert!(!semantic_capability_absent(&c, Some(&e), "right_click"));
}

#[test]
fn upstream_policy_override_guidance_never_crosses_the_private_tool_boundary() {
    let receipt = json!({"version":"2.5","ok":false,"error":{"code":"POLICY_DENIED","message":"Sensitive target blocked.","details":{"clovy_code":"computer_use_sensitive_field_blocked"},"disposition":{"delivery":"not_delivered","retry":"safe"},"recovery":{"retryable":true,"strategy":"request_explicit_policy_then_retry_original"}}});
    let result = normalize(receipt, "background");
    assert!(!result.to_string().contains("request_explicit_policy"));
    assert!(result["structuredContent"]["error"]
        .get("recovery")
        .is_none());
    assert_eq!(
        result["structuredContent"]["disposition"]["delivery"],
        "not_delivered"
    );
    assert_eq!(
        result["structuredContent"]["error"]["details"]["clovy_code"],
        "computer_use_sensitive_field_blocked"
    );
    assert!(result["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("Take over"));
    let visible = remode_failure(
        failure("native_transport_lost", "Original native cause.", true),
        "visible",
    );
    assert_eq!(
        visible["structuredContent"]["effective_modality"],
        "visible"
    );
    assert!(visible["content"][0]["text"]
        .as_str()
        .unwrap()
        .ends_with("visible."));
}

#[test]
fn an_empty_modifier_array_preserves_semantic_background_click() {
    let capture = capture();
    let element = &capture.elements[&1];
    let args = json!({"modifier":[]});
    assert!(!has_modifiers(&args));
    let argv = action_argv("click", &args, &capture, Some(element), false).unwrap();
    assert_eq!(argv, ["click", "@snapshot:e4"]);
}

#[cfg(target_os = "linux")]
#[test]
fn linux_active_window_keys_report_visible_without_changing_dispatch_policy() {
    let c = capture();
    for (name, args) in [
        ("press_key", json!({"key":"Return"})),
        ("hotkey", json!({"keys":["ctrl","a"]})),
    ] {
        let argv = action_argv(name, &args, &c, None, false).unwrap();
        assert_eq!(argv[0], "press");
        assert!(!argv.iter().any(|arg| arg == "--headed"));
        assert!(!argv.iter().any(|arg| arg == "focus-window"));
        let mode = action_modality(name, false);
        assert_eq!(mode, "visible");
        let success = normalize(json!({"version":"2.5","ok":true,"data":{}}), mode);
        assert_eq!(
            success["structuredContent"]["effective_modality"],
            "visible"
        );
        assert!(success["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("visible mode"));
        let error = remode_failure(
            failure("transport_lost", "Original native cause", true),
            mode,
        );
        assert_eq!(error["structuredContent"]["effective_modality"], "visible");
        assert!(error["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("visible"));
        assert_eq!(error["structuredContent"]["disposition"]["retry"], "unsafe");
    }
}

#[test]
fn window_keys_transport_expected_capture_identity_to_the_leased_consumer() {
    let captured = capture();
    for (name, args) in [
        ("press_key", json!({"key":"escape"})),
        ("hotkey", json!({"keys":["ctrl","f"]})),
    ] {
        for physical in [false, true] {
            let argv = action_argv(name, &args, &captured, None, physical).unwrap();
            let at = argv
                .iter()
                .position(|arg| arg == "--clovy-target-json")
                .unwrap();
            let expected: Value = serde_json::from_str(&argv[at + 1]).unwrap();
            assert_eq!(expected, captured.window.raw);
            assert_eq!(expected["pid"], 42);
            assert_eq!(expected["process_instance"], "p42:100");
            assert!(!argv.iter().any(|arg| arg == "--clovy-ref"));
        }
    }
}
#[test]
fn preparatory_focus_failure_remains_unsafe_and_cannot_escalate() {
    let native = json!({"version":"2.5","ok":false,"error":{"code":"ACTION_NOT_SUPPORTED","message":"Native input refused before Send","details":{"native_disposition":{"delivery":"not_delivered","retry":"safe"},"preparatory_effects":{"focus":{"attempted":true,"confirmed":true},"target_input":{"attempted":true}}},"disposition":{"delivery":"delivered_unverified","retry":"unsafe"}}});
    assert!(!may_escalate(&native));
    let envelope = normalize(native.clone(), "visible");
    assert_eq!(
        envelope["structuredContent"]["disposition"]["retry"],
        "unsafe"
    );
    assert_eq!(
        envelope["structuredContent"]["engine_evidence"]["error"]["details"],
        native["error"]["details"]
    );
    assert_eq!(
        envelope["structuredContent"]["engine_evidence"]["error"]["details"]["native_disposition"]
            ["retry"],
        "safe"
    );
}
