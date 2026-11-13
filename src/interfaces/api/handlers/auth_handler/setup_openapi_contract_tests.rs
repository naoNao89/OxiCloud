    //! The published contract of `POST /api/setup`, in two layers.
    //!
    //! * Source: the `#[utoipa::path]` on `setup_admin` (asserted through
    //!   `ApiDoc::openapi()`), which is what the generator projects.
    //! * Artifact: `resources/gen/openapi.json`, which is **checked in** — CI's
    //!   `openapi-spec-drift` job regenerates it and fails on any diff, so this
    //!   file, not the attribute, is what third-party clients and the generated
    //!   TypeScript SDK actually consume.
    //!
    //! Why the artifact layer is here and not in `interfaces::api`'s spec tests:
    //! those assert against `ApiDoc::openapi()` in memory and therefore cannot
    //! see a stale file — a declaration change nobody regenerated would pass
    //! them and still ship an outdated contract. No test in this repo read
    //! `resources/gen/*.json` before, so this module is next to the declaration
    //! it guards and reads the file by `CARGO_MANIFEST_DIR`.
    use utoipa::OpenApi;

    fn committed_spec() -> serde_json::Value {
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("resources/gen/openapi.json");
        let raw = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        serde_json::from_str(&raw)
            .unwrap_or_else(|e| panic!("parse {} as JSON: {e}", path.display()))
    }

    /// The shared shape of both layers: 201/400/403/429/503 declared, every
    /// error status carrying `ApiErrorBody`, and the 429 naming `Retry-After`.
    fn assert_setup_contract(spec: &serde_json::Value, source: &str) {
        let responses = spec["paths"]["/api/setup"]["post"]["responses"]
            .as_object()
            .unwrap_or_else(|| panic!("{source} has no POST /api/setup responses"));

        let declared: Vec<&str> = responses.keys().map(String::as_str).collect();
        for status in ["201", "400", "403", "429", "503"] {
            assert!(
                responses.contains_key(status),
                "{source}: POST /api/setup must declare {status}; it declares {declared:?}"
            );
        }

        for status in ["400", "403", "429", "503"] {
            assert_eq!(
                responses[status]["content"]["application/json"]["schema"]["$ref"],
                "#/components/schemas/ApiErrorBody",
                "{source}: the {status} body must be published as ApiErrorBody"
            );
        }
        assert_eq!(
            responses["201"]["content"]["application/json"]["schema"]["$ref"],
            "#/components/schemas/PublicUserDto",
            "{source}: the success body must stay PublicUserDto"
        );

        let description = responses["429"]["description"].as_str().unwrap_or_default();
        assert!(
            description.contains("Retry-After"),
            "{source}: the 429 must say where the wait is carried; got {description:?}"
        );

        for name in ["ApiErrorBody", "PublicUserDto"] {
            assert!(
                spec["components"]["schemas"].get(name).is_some(),
                "{source}: components.schemas.{name} is referenced but not published"
            );
        }
    }

    /// The attribute itself. GREEN as soon as the `#[utoipa::path]` above is
    /// edited: it proves the declaration is well-formed and projects the refs
    /// the artifact test then looks for.
    #[test]
    fn setup_admin_attribute_declares_the_contract() {
        let spec = serde_json::to_value(crate::interfaces::api::ApiDoc::openapi())
            .expect("ApiDoc serialises");
        assert_setup_contract(&spec, "ApiDoc::openapi()");
    }

    /// The checked-in artifact, read from `resources/gen/openapi.json`.
    ///
    /// Before the spec was regenerated this test was RED: HEAD's file declared
    /// only 201/403/503 for `/api/setup` (403 and 503 did carry the
    /// `ApiErrorBody` ref; 400 and 429 were absent entirely). In this tree the
    /// artifact HAS been regenerated (`just gen-openapi`, done by the build side),
    /// so the test is GREEN; from here on it fails whenever a change to the
    /// attribute is not accompanied by a regenerated spec, or whenever a
    /// regeneration drops a documented status — instead of the client contract
    /// regressing unnoticed.
    ///
    /// Text-only note: this comment is `cfg(test)`-level prose. It changes no
    /// source line, binary or code digest, so the arm builds do not need re-running
    /// for it.
    #[test]
    fn committed_spec_declares_the_setup_contract() {
        let spec = committed_spec();
        assert_setup_contract(&spec, "resources/gen/openapi.json");
    }
