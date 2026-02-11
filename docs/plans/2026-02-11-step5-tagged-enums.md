# Tagged Enums Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.
> **REQUIRED SKILL:** Use `ms-rust` skill BEFORE editing any `.rs` file.

**Goal:** Support all 4 serde enum representations (externally tagged, internally tagged, adjacently tagged, untagged) with full C# converter generation, achieving parity with ts-rs.

**Architecture:** Extend the existing IR with `EnumTagging` + `TaggedVariant` + `TaggedVariantData` types. Parse `#[serde(tag, content, untagged)]` in `ContainerAttr`. Split `codegen.rs` into a `codegen/` module. Generate abstract record + nested sealed derived records + custom `JsonConverter<T>` (or `[JsonPolymorphic]` for internally tagged + STJ + C# 11+). Leverage `CSharpVersion` for file-scoped namespaces (C# 10+), `required` (C# 11+), and native polymorphic attrs (C# 11+).

**Tech Stack:** Rust 2024 edition, `syn`/`quote`/`proc-macro2`, strict clippy pedantic lints.

**Variant support:** Unit, Struct (named fields), Newtype (single wrapped type). Tuple variants rejected with compile error.

---

## Task 1: Parse `tag`, `content`, `untagged` in ContainerAttr

**Files:**
- Modify: `crates/csharp-rs-macros/src/attr/container.rs`

**Step 1: Write failing tests**

Add to `container.rs` tests module:

```rust
#[test]
fn parse_serde_tag() {
    let attrs: Vec<Attribute> = vec![parse_quote!(#[serde(tag = "type")])];
    let container = ContainerAttr::from_attrs(&attrs).unwrap();
    assert_eq!(container.tag.as_deref(), Some("type"));
}

#[test]
fn parse_serde_tag_and_content() {
    let attrs: Vec<Attribute> = vec![parse_quote!(#[serde(tag = "t", content = "c")])];
    let container = ContainerAttr::from_attrs(&attrs).unwrap();
    assert_eq!(container.tag.as_deref(), Some("t"));
    assert_eq!(container.content.as_deref(), Some("c"));
}

#[test]
fn parse_serde_untagged() {
    let attrs: Vec<Attribute> = vec![parse_quote!(#[serde(untagged)])];
    let container = ContainerAttr::from_attrs(&attrs).unwrap();
    assert!(container.untagged);
}

#[test]
fn parse_combined_tag_and_rename_all() {
    let attrs: Vec<Attribute> = vec![
        parse_quote!(#[serde(tag = "kind", rename_all = "camelCase")]),
    ];
    let container = ContainerAttr::from_attrs(&attrs).unwrap();
    assert_eq!(container.tag.as_deref(), Some("kind"));
    assert_eq!(container.rename_all, Some(Inflection::Camel));
}

#[test]
fn no_tag_defaults_to_none() {
    let attrs: Vec<Attribute> = vec![];
    let container = ContainerAttr::from_attrs(&attrs).unwrap();
    assert!(container.tag.is_none());
    assert!(container.content.is_none());
    assert!(!container.untagged);
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test -p csharp-rs-macros attr::container::tests`
Expected: FAIL — `tag`, `content`, `untagged` fields don't exist on `ContainerAttr`.

**Step 3: Implement**

Add 3 fields to `ContainerAttr`:

```rust
pub struct ContainerAttr {
    pub rename_all: Option<Inflection>,
    pub namespace: Option<String>,
    pub export: bool,
    pub export_to: Option<String>,
    pub tag: Option<String>,         // NEW
    pub content: Option<String>,     // NEW
    pub untagged: bool,              // NEW
}
```

In `parse_serde()`, add branches after the `rename_all` match:

```rust
if meta.path.is_ident("tag") {
    let value = meta.value()?;
    let lit: Lit = value.parse()?;
    if let Lit::Str(lit_str) = lit {
        self.tag = Some(lit_str.value());
    }
} else if meta.path.is_ident("content") {
    let value = meta.value()?;
    let lit: Lit = value.parse()?;
    if let Lit::Str(lit_str) = lit {
        self.content = Some(lit_str.value());
    }
} else if meta.path.is_ident("untagged") {
    self.untagged = true;
}
```

These use `else if` — serde attributes are already silently ignored for unknown attrs, so this just captures 3 more.

**Step 4: Run tests to verify they pass**

Run: `cargo test -p csharp-rs-macros attr::container::tests`
Expected: ALL PASS

**Step 5: Run full workspace tests (no regressions)**

Run: `cargo test --workspace && cargo clippy --workspace`
Expected: ALL PASS, no clippy warnings.

**Step 6: Commit**

```
feat(attr): parse serde tag, content, and untagged container attributes
```

---

## Task 2: Add IR types and update dispatch

**Files:**
- Modify: `crates/csharp-rs-macros/src/types/mod.rs`

**Step 1: Write failing tests**

Add to `types/mod.rs` tests module:

```rust
#[test]
fn enum_with_struct_variant_and_tag_succeeds() {
    let input: DeriveInput = parse_quote! {
        #[serde(tag = "type")]
        enum Message {
            Request { id: String },
            Quit,
        }
    };
    let result = process_input(&input, &default_config());
    assert!(result.is_ok(), "tagged enum should succeed: {}", result.unwrap_err());
    let ir = result.unwrap();
    assert_eq!(ir.csharp_name, "Message");
    assert!(matches!(ir.kind, DerivedCSharpKind::TaggedEnum { .. }));
}

#[test]
fn enum_with_data_variant_no_tag_defaults_to_external() {
    let input: DeriveInput = parse_quote! {
        enum Message {
            Text(String),
            Quit,
        }
    };
    let result = process_input(&input, &default_config());
    assert!(result.is_ok());
    let ir = result.unwrap();
    match &ir.kind {
        DerivedCSharpKind::TaggedEnum { tagging, .. } => {
            assert!(matches!(tagging, EnumTagging::External));
        }
        _ => panic!("expected TaggedEnum kind"),
    }
}

#[test]
fn enum_with_tuple_variant_errors() {
    let input: DeriveInput = parse_quote! {
        #[serde(tag = "type")]
        enum Message {
            Data(String, i32),
        }
    };
    let result = process_input(&input, &default_config());
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("tuple variants"),
        "should mention tuple variants: {err}"
    );
}

#[test]
fn all_unit_with_tag_becomes_tagged_enum() {
    let input: DeriveInput = parse_quote! {
        #[serde(tag = "type")]
        enum Status {
            Active,
            Inactive,
        }
    };
    let result = process_input(&input, &default_config());
    assert!(result.is_ok());
    assert!(matches!(result.unwrap().kind, DerivedCSharpKind::TaggedEnum { .. }));
}

#[test]
fn all_unit_without_tag_stays_simple_enum() {
    let input: DeriveInput = parse_quote! {
        enum Color {
            Red,
            Green,
        }
    };
    let result = process_input(&input, &default_config());
    assert!(result.is_ok());
    assert!(matches!(result.unwrap().kind, DerivedCSharpKind::Enum(_)));
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test -p csharp-rs-macros types::tests`
Expected: FAIL — `DerivedCSharpKind::TaggedEnum` doesn't exist, `EnumTagging` doesn't exist.

**Step 3: Implement IR types**

Add to `types/mod.rs` (after existing `CSharpVariant`):

```rust
/// How the enum is tagged in JSON (from serde attributes).
#[derive(Debug)]
pub enum EnumTagging {
    /// Default serde: `{"VariantName": data}` / `"UnitVariant"`.
    External,
    /// `#[serde(tag = "...")]`: discriminator merged into object.
    Internal { tag: String },
    /// `#[serde(tag = "...", content = "...")]`: separate tag and content keys.
    Adjacent { tag: String, content: String },
    /// `#[serde(untagged)]`: no discriminator, try each variant.
    Untagged,
}

/// Data carried by a variant in a tagged enum.
#[derive(Debug)]
pub enum TaggedVariantData {
    /// No data (unit variant).
    Unit,
    /// Wraps a single type: `Variant(Type)`.
    Newtype { type_expr: TokenStream },
    /// Named fields: `Variant { field: Type, ... }`.
    Struct(Vec<CSharpField>),
}

/// A variant in a tagged enum.
#[derive(Debug)]
pub struct TaggedVariant {
    /// C# record name (PascalCase, from Rust variant ident).
    pub csharp_name: String,
    /// JSON discriminator value (after `rename_all` / per-variant `rename`).
    pub json_name: String,
    /// Data carried by this variant.
    pub data: TaggedVariantData,
}
```

Add to `DerivedCSharpKind`:

```rust
pub enum DerivedCSharpKind {
    Record(Vec<CSharpField>),
    Enum(Vec<CSharpVariant>),
    TaggedEnum {
        tagging: EnumTagging,
        variants: Vec<TaggedVariant>,
    },
}
```

**Step 4: Add `tagged_enum` module and update dispatch**

Create `crates/csharp-rs-macros/src/types/tagged_enum.rs` as a stub:

```rust
//! Tagged enum processing for C# code generation.

use crate::attr::container::ContainerAttr;
use crate::config::CSharpConfig;
use crate::types::DerivedCSharp;
use syn::{DataEnum, DeriveInput};

pub fn tagged_enum(
    _input: &DeriveInput,
    _enum_data: &DataEnum,
    _container: &ContainerAttr,
    _config: &CSharpConfig,
) -> syn::Result<DerivedCSharp> {
    todo!("tagged_enum IR builder")
}
```

Register module in `types/mod.rs`:

```rust
pub mod tagged_enum;
```

Update `process_input()` dispatch for `Data::Enum`:

```rust
Data::Enum(enum_data) => {
    let has_data_variants = enum_data.variants.iter().any(|v| !v.fields.is_empty());
    let has_explicit_tagging =
        container.tag.is_some() || container.content.is_some() || container.untagged;

    if has_data_variants || has_explicit_tagging {
        tagged_enum::tagged_enum(input, enum_data, &container, config)
    } else {
        simple_enum::simple_enum(input, enum_data, &container, config)
    }
},
```

Update the existing `enum_with_tuple_variant_errors` test expectation — it currently expects the error from `simple_enum` ("only unit variants"), but now the enum routes to `tagged_enum` instead. The new error will be about tuple variants. Adjust the test accordingly.

**Step 5: Run tests to verify they pass**

Run: `cargo test -p csharp-rs-macros types::tests`
Expected: New tests pass (tagged enum routing works). Some will still fail if they need the tagged_enum builder — those tests hit `todo!()`. Handle in Task 3.

Note: the `todo!()` will panic at runtime, but the tests that check `result.is_ok()` on tagged enums will fail. That's expected — Task 3 implements the builder.

**Step 6: Run clippy**

Run: `cargo clippy --workspace`
Expected: CLEAN (ignore the `todo!` warning if present).

**Step 7: Commit**

```
feat(types): add TaggedEnum IR types and update enum dispatch
```

---

## Task 3: Implement tagged_enum IR builder

**Files:**
- Modify: `crates/csharp-rs-macros/src/types/tagged_enum.rs`

**Step 1: Write failing tests in tagged_enum.rs**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::attr::container::ContainerAttr;
    use crate::types::{EnumTagging, TaggedVariantData, DerivedCSharpKind};
    use syn::parse_quote;

    fn default_config() -> CSharpConfig {
        CSharpConfig::default()
    }

    fn process(input: &DeriveInput) -> DerivedCSharp {
        let container = ContainerAttr::from_attrs(&input.attrs).unwrap();
        let syn::Data::Enum(ref enum_data) = input.data else { panic!("expected enum") };
        tagged_enum(input, enum_data, &container, &default_config()).unwrap()
    }

    fn extract_variants(ir: &DerivedCSharp) -> (&EnumTagging, &[TaggedVariant]) {
        match &ir.kind {
            DerivedCSharpKind::TaggedEnum { tagging, variants } => (tagging, variants),
            _ => panic!("expected TaggedEnum kind"),
        }
    }

    #[test]
    fn internal_tagging_parsed() {
        let input: DeriveInput = parse_quote! {
            #[serde(tag = "type")]
            enum Message {
                Request { id: String },
                Quit,
            }
        };
        let ir = process(&input);
        let (tagging, variants) = extract_variants(&ir);
        assert!(matches!(tagging, EnumTagging::Internal { tag } if tag == "type"));
        assert_eq!(variants.len(), 2);
    }

    #[test]
    fn adjacent_tagging_parsed() {
        let input: DeriveInput = parse_quote! {
            #[serde(tag = "t", content = "c")]
            enum Block {
                Para(String),
                Code { lang: String, body: String },
            }
        };
        let ir = process(&input);
        let (tagging, variants) = extract_variants(&ir);
        assert!(matches!(tagging, EnumTagging::Adjacent { tag, content } if tag == "t" && content == "c"));
        assert_eq!(variants.len(), 2);
    }

    #[test]
    fn external_tagging_default() {
        let input: DeriveInput = parse_quote! {
            enum Message {
                Text(String),
                Quit,
            }
        };
        let ir = process(&input);
        let (tagging, _) = extract_variants(&ir);
        assert!(matches!(tagging, EnumTagging::External));
    }

    #[test]
    fn untagged_parsed() {
        let input: DeriveInput = parse_quote! {
            #[serde(untagged)]
            enum Data {
                Text(String),
                Number { value: f64 },
            }
        };
        let ir = process(&input);
        let (tagging, _) = extract_variants(&ir);
        assert!(matches!(tagging, EnumTagging::Untagged));
    }

    #[test]
    fn struct_variant_produces_struct_data() {
        let input: DeriveInput = parse_quote! {
            #[serde(tag = "type")]
            enum Event {
                Click { x: i32, y: i32 },
            }
        };
        let ir = process(&input);
        let (_, variants) = extract_variants(&ir);
        assert_eq!(variants[0].csharp_name, "Click");
        assert!(matches!(&variants[0].data, TaggedVariantData::Struct(fields) if fields.len() == 2));
    }

    #[test]
    fn newtype_variant_produces_newtype_data() {
        let input: DeriveInput = parse_quote! {
            #[serde(tag = "t", content = "c")]
            enum Value {
                Text(String),
            }
        };
        let ir = process(&input);
        let (_, variants) = extract_variants(&ir);
        assert!(matches!(&variants[0].data, TaggedVariantData::Newtype { .. }));
    }

    #[test]
    fn unit_variant_produces_unit_data() {
        let input: DeriveInput = parse_quote! {
            #[serde(tag = "type")]
            enum Message {
                Quit,
            }
        };
        let ir = process(&input);
        let (_, variants) = extract_variants(&ir);
        assert!(matches!(&variants[0].data, TaggedVariantData::Unit));
    }

    #[test]
    fn tuple_variant_rejected() {
        let input: DeriveInput = parse_quote! {
            #[serde(tag = "type")]
            enum Msg {
                Data(String, i32),
            }
        };
        let container = ContainerAttr::from_attrs(&input.attrs).unwrap();
        let syn::Data::Enum(ref enum_data) = input.data else { panic!() };
        let result = tagged_enum(&input, enum_data, &container, &default_config());
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("tuple variants"));
    }

    #[test]
    fn content_without_tag_errors() {
        let input: DeriveInput = parse_quote! {
            #[serde(content = "c")]
            enum Msg {
                Text(String),
            }
        };
        let container = ContainerAttr::from_attrs(&input.attrs).unwrap();
        let syn::Data::Enum(ref enum_data) = input.data else { panic!() };
        let result = tagged_enum(&input, enum_data, &container, &default_config());
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("content"));
    }

    #[test]
    fn rename_all_applies_to_variant_json_names() {
        let input: DeriveInput = parse_quote! {
            #[serde(tag = "type", rename_all = "camelCase")]
            enum Event {
                UserLogin { user_id: String },
                SessionEnd,
            }
        };
        let ir = process(&input);
        let (_, variants) = extract_variants(&ir);
        assert_eq!(variants[0].json_name, "userLogin");
        assert_eq!(variants[0].csharp_name, "UserLogin");
        assert_eq!(variants[1].json_name, "sessionEnd");
    }

    #[test]
    fn per_variant_rename_overrides_rename_all() {
        let input: DeriveInput = parse_quote! {
            #[serde(tag = "type", rename_all = "camelCase")]
            enum Event {
                #[serde(rename = "CLICK")]
                Click { x: i32 },
                Move { dx: i32 },
            }
        };
        let ir = process(&input);
        let (_, variants) = extract_variants(&ir);
        assert_eq!(variants[0].json_name, "CLICK");
        assert_eq!(variants[1].json_name, "move");
    }

    #[test]
    fn skip_variant_excluded() {
        let input: DeriveInput = parse_quote! {
            #[serde(tag = "type")]
            enum Message {
                Text(String),
                #[serde(skip)]
                Internal,
                Quit,
            }
        };
        let ir = process(&input);
        let (_, variants) = extract_variants(&ir);
        assert_eq!(variants.len(), 2);
        assert_eq!(variants[0].csharp_name, "Text");
        assert_eq!(variants[1].csharp_name, "Quit");
    }

    #[test]
    fn struct_variant_fields_respect_field_attrs() {
        let input: DeriveInput = parse_quote! {
            #[serde(tag = "type")]
            enum Event {
                Click {
                    #[serde(rename = "posX")]
                    x: i32,
                    #[serde(skip)]
                    internal: String,
                    y: i32,
                },
            }
        };
        let ir = process(&input);
        let (_, variants) = extract_variants(&ir);
        match &variants[0].data {
            TaggedVariantData::Struct(fields) => {
                assert_eq!(fields.len(), 2);
                assert_eq!(fields[0].json_name, "posX");
                assert_eq!(fields[0].csharp_property_name, "X");
                assert_eq!(fields[1].csharp_property_name, "Y");
            }
            _ => panic!("expected Struct data"),
        }
    }
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test -p csharp-rs-macros types::tagged_enum::tests`
Expected: FAIL — `tagged_enum()` is `todo!()`.

**Step 3: Implement `tagged_enum()`**

The function follows the same pattern as `simple_enum()` and `named_struct()`:
1. Resolve `EnumTagging` from container attrs
2. Iterate variants, parse `FieldAttr` per variant
3. For each variant: inspect `Fields` to determine `TaggedVariantData`
   - `Fields::Unit` → `TaggedVariantData::Unit`
   - `Fields::Unnamed` with exactly 1 field → `TaggedVariantData::Newtype { type_expr }`
   - `Fields::Unnamed` with 2+ fields → error (tuple variants not supported)
   - `Fields::Named` → `TaggedVariantData::Struct(fields)` — reuse field processing logic from `named.rs`
4. Build `DerivedCSharp` with `DerivedCSharpKind::TaggedEnum { tagging, variants }`

Key: Extract the shared field-processing logic from `named.rs` (the part that iterates `FieldsNamed`, applies `rename_all`, `FieldAttr`, `analyze_type`) into a reusable function. Either move it to a helper in `types/mod.rs` or call `named.rs` functions. The `analyze_type`, `type_to_token_expr`, `extract_option_inner` functions in `named.rs` are already standalone — make them `pub(crate)` so `tagged_enum.rs` can reuse them.

**Step 4: Run tests**

Run: `cargo test -p csharp-rs-macros types::tagged_enum::tests`
Expected: ALL PASS

**Step 5: Run full test suite**

Run: `cargo test --workspace && cargo clippy --workspace`
Expected: ALL PASS. Existing tests still work. The dispatch tests from Task 2 now work too (no more `todo!()`). However, codegen tests for tagged enums will fail since codegen doesn't handle `TaggedEnum` yet — that's fine if those integration tests don't exist yet.

**Step 6: Commit**

```
feat(types): implement tagged enum IR builder with variant data support
```

---

## Task 4: Split codegen.rs into codegen/ module

**Files:**
- Delete: `crates/csharp-rs-macros/src/codegen.rs`
- Create: `crates/csharp-rs-macros/src/codegen/mod.rs`
- Create: `crates/csharp-rs-macros/src/codegen/record.rs`
- Create: `crates/csharp-rs-macros/src/codegen/simple_enum.rs`
- Create: `crates/csharp-rs-macros/src/codegen/tagged_enum.rs` (stub)
- Modify: `crates/csharp-rs-macros/src/lib.rs` (module declaration stays `mod codegen` — no change needed)

This is a **pure refactor**: extract `build_record_definition` into `codegen/record.rs`, `build_enum_definition` into `codegen/simple_enum.rs`. The dispatch (`build_definition`, `build_dependencies`, `build_export_test`, `into_token_stream`) stays in `codegen/mod.rs`.

**Step 1: Create directory and move code**

`codegen/mod.rs` keeps:
- `impl DerivedCSharp { into_token_stream, build_definition, build_dependencies, build_export_test }`
- `build_definition` dispatches to `record::build_record_definition`, `simple_enum::build_enum_definition`
- Add stub arm for `TaggedEnum` that calls `tagged_enum::build_tagged_enum_definition`
- All existing tests

`codegen/record.rs`:
- Contains `pub(crate) fn build_record_definition(...)` (extracted from the method)

`codegen/simple_enum.rs`:
- Contains `pub(crate) fn build_enum_definition(...)` (extracted from the method)

`codegen/tagged_enum.rs`:
- Stub: `pub(crate) fn build_tagged_enum_definition(...) -> TokenStream { todo!() }`

**Step 2: Run all tests — no behavior change**

Run: `cargo test --workspace && cargo clippy --workspace`
Expected: ALL existing 148 tests PASS. This is a pure refactor.

**Step 3: Commit**

```
refactor(codegen): split codegen.rs into module with record, simple_enum, tagged_enum
```

---

## Task 5: Codegen — Type hierarchy (abstract record + derived records)

**Files:**
- Modify: `crates/csharp-rs-macros/src/codegen/tagged_enum.rs`
- Modify: `crates/csharp-rs-macros/src/codegen/mod.rs` (add tests)

This task implements the **type hierarchy** generation — the abstract record base + nested derived records — shared by all 4 tagging modes. The converter is NOT generated yet; this task produces a compilable C# type hierarchy without JSON serialization support.

**Step 1: Write failing test**

In `codegen/mod.rs` tests (or a new test module in `codegen/tagged_enum.rs`):

```rust
#[test]
fn tagged_enum_stj_internally_tagged_type_hierarchy() {
    let ir = DerivedCSharp {
        rust_ident: quote::format_ident!("Message"),
        csharp_name: String::from("Message"),
        namespace: CSharpNamespace::new("Test.Ns").unwrap(),
        kind: DerivedCSharpKind::TaggedEnum {
            tagging: EnumTagging::Internal { tag: String::from("type") },
            variants: vec![
                TaggedVariant {
                    csharp_name: String::from("Request"),
                    json_name: String::from("Request"),
                    data: TaggedVariantData::Struct(vec![
                        CSharpField {
                            csharp_property_name: String::from("Id"),
                            json_name: String::from("id"),
                            type_expr: quote! { <String as csharp_rs::CSharp>::csharp_name() },
                            is_optional: false,
                        },
                    ]),
                },
                TaggedVariant {
                    csharp_name: String::from("Text"),
                    json_name: String::from("Text"),
                    data: TaggedVariantData::Newtype {
                        type_expr: quote! { <String as csharp_rs::CSharp>::csharp_name() },
                    },
                },
                TaggedVariant {
                    csharp_name: String::from("Quit"),
                    json_name: String::from("Quit"),
                    data: TaggedVariantData::Unit,
                },
            ],
        },
        export: false,
        export_to: None,
    };

    let config = CSharpConfig {
        target: CSharpVersion::CSharp11,
        ..CSharpConfig::default()
    };
    let tokens = ir.into_token_stream(&config).to_string();

    // Type hierarchy
    assert!(tokens.contains("abstract record Message"), "missing abstract record:\n{tokens}");
    assert!(tokens.contains("sealed record Request"), "missing Request variant:\n{tokens}");
    assert!(tokens.contains("sealed record Text"), "missing Text variant:\n{tokens}");
    assert!(tokens.contains("sealed record Quit"), "missing Quit variant:\n{tokens}");
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p csharp-rs-macros codegen::tests::tagged_enum`
Expected: FAIL — `todo!()` panic.

**Step 3: Implement type hierarchy generation**

In `codegen/tagged_enum.rs`, implement `build_tagged_enum_definition()` that:

1. Chooses the serialization strategy (native `[JsonPolymorphic]` vs custom converter) based on config
2. Builds the abstract record + nested derived records as a string at macro time (same pattern as `build_enum_definition` — enum data is fully known at macro time for unit/struct variants)
3. For struct variants: generates properties with JSON attributes (same as records)
4. For newtype variants: generates a single `Value` property with JSON attribute
5. For unit variants: generates empty sealed record
6. Applies version-dependent features:
   - C# 10+: file-scoped namespace (`namespace X;` instead of `namespace X { }`)
   - C# 11+: `required` modifier on non-optional properties

Start with **internally tagged + STJ + C# 11+** (the native `[JsonPolymorphic]` path — simplest, no converter needed).

Expected C# output for this test case:

```csharp
// <auto-generated/>
using System.Text.Json.Serialization;

namespace Test.Ns;

[JsonPolymorphic(TypeDiscriminatorPropertyName = "type")]
[JsonDerivedType(typeof(Message.Request), "Request")]
[JsonDerivedType(typeof(Message.Text), "Text")]
[JsonDerivedType(typeof(Message.Quit), "Quit")]
public abstract record Message
{
    public sealed record Request : Message
    {
        [JsonPropertyName("id")]
        public required string Id { get; init; }
    }

    public sealed record Text : Message
    {
        [JsonPropertyName("Value")]
        public required string Value { get; init; }
    }

    public sealed record Quit : Message;
}
```

Note: for the `TokenStream` approach, since newtype/struct variant field types are resolved at compile time (via `<T as CSharp>::csharp_name()`), the codegen needs to use the `quote!` pattern with runtime string building (same as `build_record_definition`), not pure strings like `build_enum_definition`.

**Step 4: Run tests**

Run: `cargo test -p csharp-rs-macros codegen`
Expected: New test PASSES. Existing codegen tests still PASS.

**Step 5: Add more codegen tests**

Test the other combinations incrementally:
- Newtype variant property has `Value` property name
- Unit variant generates empty record with semicolon (not braces)
- C# 9: block-scoped namespace, no `required`, custom converter placeholder
- C# 10: file-scoped namespace, no `required`
- Newtonsoft serializer attributes

**Step 6: Commit**

```
feat(codegen): generate tagged enum type hierarchy with version-dependent features
```

---

## Task 6: Codegen — Internally tagged converter (STJ custom + Newtonsoft)

**Files:**
- Modify: `crates/csharp-rs-macros/src/codegen/tagged_enum.rs`

For C# 9-10 with STJ, and ALL versions with Newtonsoft, generate a custom `JsonConverter<T>` nested inside the abstract record.

**Step 1: Write failing test for STJ C# 9**

```rust
#[test]
fn internally_tagged_stj_csharp9_has_converter() {
    // Build IR with internally tagged enum...
    let config = CSharpConfig {
        target: CSharpVersion::CSharp9,
        serializer: Serializer::SystemTextJson,
        ..CSharpConfig::default()
    };
    let tokens = ir.into_token_stream(&config).to_string();

    assert!(tokens.contains("JsonConverter"), "should have converter:\n{tokens}");
    assert!(tokens.contains("MessageConverter"), "should have named converter:\n{tokens}");
    assert!(!tokens.contains("JsonPolymorphic"), "C# 9 should NOT use JsonPolymorphic:\n{tokens}");
}
```

**Step 2: Implement STJ converter for internally tagged**

The converter pattern (as designed in brainstorming):

**Read**: `JsonDocument.ParseValue(ref reader)` → `root.GetProperty("type").GetString()` → switch → construct variant manually from properties.

**Write**: `writer.WriteStartObject()` → `writer.WriteString("type", "VariantName")` → manual property writes via `JsonSerializer.Serialize(writer, value, options)` per field → `writer.WriteEndObject()`.

Key implementation detail: for struct variants, iterate the `CSharpField` list and generate a write per field. For newtype variants, write the single `Value` property. For unit variants, just the tag.

**Step 3: Write failing test for Newtonsoft internally tagged**

Same structure but with Newtonsoft API:
- `JObject.Load(reader)` / `JObject.WriteTo(writer)`
- `[JsonConverter(typeof(MessageConverter))]` on the abstract record
- Converter extends `JsonConverter<Message>` with `ReadJson`/`WriteJson`

**Step 4: Implement Newtonsoft converter**

Same dispatch logic, different API calls.

**Step 5: Run tests**

Run: `cargo test -p csharp-rs-macros codegen`
Expected: ALL PASS.

**Step 6: Commit**

```
feat(codegen): generate custom JsonConverter for internally tagged enums
```

---

## Task 7: Codegen — Externally tagged converter

**Files:**
- Modify: `crates/csharp-rs-macros/src/codegen/tagged_enum.rs`

Same pattern as Task 6, but for externally tagged:

**Read**: Check `reader.TokenType` — if `String` → unit variant; if `StartObject` → parse single-property object, key = variant name, value = content.

**Write**: Unit variants → `writer.WriteStringValue("Quit")`. Data variants → `writer.WriteStartObject()` → `writer.WritePropertyName("VariantName")` → write content → `writer.WriteEndObject()`.

Tests should cover: struct variant, newtype variant, unit variant, mixed.

**Commit:**

```
feat(codegen): generate JsonConverter for externally tagged enums
```

---

## Task 8: Codegen — Adjacently tagged converter

**Files:**
- Modify: `crates/csharp-rs-macros/src/codegen/tagged_enum.rs`

**Read**: `root.GetProperty("t").GetString()` for tag. `root.GetProperty("c")` for content (absent for unit variants).

**Write**: `writer.WriteStartObject()` → `writer.WriteString("t", "...")` → for data variants: `writer.WritePropertyName("c")` → write content → `writer.WriteEndObject()`.

**Commit:**

```
feat(codegen): generate JsonConverter for adjacently tagged enums
```

---

## Task 9: Codegen — Untagged converter

**Files:**
- Modify: `crates/csharp-rs-macros/src/codegen/tagged_enum.rs`

The hardest mode. No discriminator — try each variant in declaration order.

**Read**: `JsonDocument.ParseValue(ref reader)` → `GetRawText()` → try each variant in order, catch `Exception` on failure, first success wins.

**Write**: No tag to write — just serialize the variant's content directly.

For struct variants: `writer.WriteStartObject()` → property writes → `writer.WriteEndObject()`.
For newtype variants: `JsonSerializer.Serialize(writer, value.Value, options)`.
For unit variants: `writer.WriteNullValue()` (serde serializes untagged unit variants as `null`).

**Commit:**

```
feat(codegen): generate JsonConverter for untagged enums
```

---

## Task 10: Version-dependent features for existing codegen

**Files:**
- Modify: `crates/csharp-rs-macros/src/codegen/record.rs`
- Modify: `crates/csharp-rs-macros/src/codegen/simple_enum.rs`
- Modify: `crates/csharp-rs-macros/src/codegen/mod.rs`

Apply C# version features to ALL codegen kinds (not just tagged enums):
- **C# 10+**: File-scoped namespaces (`namespace X;` + no outer braces + reduce indent by 4 spaces)
- **C# 11+**: `required` modifier on non-optional record properties

This requires passing `CSharpVersion` through to the codegen functions.

**Step 1: Write tests for records with C# 10+/11+**

```rust
#[test]
fn record_csharp10_file_scoped_namespace() {
    // ... build record IR, config with CSharp10
    let tokens = ir.into_token_stream(&config).to_string();
    assert!(tokens.contains("namespace Test.Ns;"), "should use file-scoped namespace");
    assert!(!tokens.contains("namespace Test.Ns\n{"), "should NOT have block namespace");
}

#[test]
fn record_csharp11_required_properties() {
    // ... build record IR, config with CSharp11
    let tokens = ir.into_token_stream(&config).to_string();
    assert!(tokens.contains("public required string"), "non-optional should be required");
}
```

**Step 2: Implement**

Add `target: CSharpVersion` parameter to `build_record_definition` and `build_enum_definition`. Branch on version for namespace style and property modifiers.

**Step 3: Commit**

```
feat(codegen): add version-dependent file-scoped namespaces and required modifier
```

---

## Task 11: Integration tests

**Files:**
- Create: `crates/csharp-rs/tests/derive_tagged_enum.rs`

End-to-end tests that use `#[derive(CSharp)]` and check the generated C# string output.

Test cases (each a separate `#[derive(CSharp)]` enum + assertions):

1. **Internally tagged — struct + unit variants**
2. **Internally tagged — newtype + unit variants**
3. **Internally tagged — with `rename_all`**
4. **Internally tagged — with per-variant rename + skip**
5. **Externally tagged — struct + newtype + unit variants**
6. **Adjacently tagged — struct + newtype + unit variants**
7. **Untagged — struct + newtype variants**
8. **Namespace override on tagged enum**
9. **Dependencies include variant field types**

Example:

```rust
use csharp_rs::CSharp;

#[derive(CSharp)]
#[serde(tag = "type")]
enum Message {
    Request { id: String, method: String },
    Quit,
}

#[test]
fn internally_tagged_has_abstract_record() {
    let def = Message::csharp_definition();
    assert!(def.contains("public abstract record Message"), "missing abstract record:\n{def}");
}

#[test]
fn internally_tagged_has_derived_records() {
    let def = Message::csharp_definition();
    assert!(def.contains("public sealed record Request : Message"), "missing Request:\n{def}");
    assert!(def.contains("public sealed record Quit : Message"), "missing Quit:\n{def}");
}

#[test]
fn internally_tagged_has_discriminator_handling() {
    let def = Message::csharp_definition();
    // Default config is C# 9 + STJ → custom converter
    assert!(def.contains("MessageConverter"), "missing converter:\n{def}");
}
```

**Commit:**

```
feat(tests): add integration tests for tagged enum derive
```

---

## Task 12: Update dependencies, cleanup, and final verification

**Files:**
- Modify: `crates/csharp-rs-macros/src/codegen/mod.rs` (update `build_dependencies` for TaggedEnum)
- Modify: auto-memory files

**Step 1: Implement `build_dependencies` for TaggedEnum**

Tagged enum dependencies = all type expressions from all struct variant fields + newtype variant types.

```rust
DerivedCSharpKind::TaggedEnum { variants, .. } => {
    let type_exprs: Vec<&TokenStream> = variants.iter().flat_map(|v| match &v.data {
        TaggedVariantData::Struct(fields) => fields.iter().map(|f| &f.type_expr).collect::<Vec<_>>(),
        TaggedVariantData::Newtype { type_expr } => vec![type_expr],
        TaggedVariantData::Unit => vec![],
    }).collect();

    if type_exprs.is_empty() {
        quote! { Vec::new() }
    } else {
        quote! { vec![#(#type_exprs),*] }
    }
}
```

**Step 2: Full verification**

Run:
```bash
cargo test --workspace
cargo clippy --workspace
cargo llvm-cov --workspace --fail-under-lines 98
```

All must pass. Target: ~230+ tests, 98%+ coverage.

**Step 3: Commit**

```
feat(codegen): implement dependencies for tagged enums and final cleanup
```

---

## Verification checklist

```bash
cargo test --workspace                           # all tests pass
cargo clippy --workspace                         # no warnings
cargo llvm-cov --workspace --html --open         # visual coverage review
cargo llvm-cov --workspace --fail-under-lines 98 # coverage threshold
```

## Expected test count

| Module | Before | After |
|--------|--------|-------|
| csharp-rs unit tests | 20 | 20 |
| csharp-rs integration tests | 35 | ~55 |
| csharp-rs-macros unit tests | 92 | ~170 |
| doctests | 1 | 1 |
| **Total** | **148** | **~246** |
