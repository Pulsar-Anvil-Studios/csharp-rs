//! Integration tests for generic type parameter support.

#![expect(dead_code, reason = "test structs are only used via derive macro")]

use csharp_rs::{CSharp, CSharpVersion, Config, Serializer};

// -----------------------------------------------------------------------
// Generic structs — basic
// -----------------------------------------------------------------------

#[derive(CSharp)]
struct Container<T> {
    item: T,
    count: u32,
}

#[derive(CSharp)]
struct Pair<A, B> {
    first: A,
    second: B,
}

#[test]
fn generic_struct_name_with_concrete_type() {
    let cfg = Config::default();
    assert_eq!(Container::<String>::csharp_name(&cfg), "Container<string>");
}

#[test]
fn generic_struct_name_two_params() {
    let cfg = Config::default();
    assert_eq!(Pair::<String, i32>::csharp_name(&cfg), "Pair<string, int>");
}

#[test]
fn generic_struct_definition_has_type_param() {
    let cfg = Config::default().with_target(CSharpVersion::CSharp10);
    let def = Container::<String>::csharp_definition(&cfg);
    assert!(
        def.contains("Container<T>"),
        "should have Container<T> in definition:\n{def}"
    );
}

#[test]
fn generic_struct_definition_has_generic_field() {
    let cfg = Config::default().with_target(CSharpVersion::CSharp10);
    let def = Container::<String>::csharp_definition(&cfg);
    assert!(def.contains("T Item"), "field type should be T:\n{def}");
}

#[test]
fn generic_struct_definition_has_concrete_field() {
    let cfg = Config::default().with_target(CSharpVersion::CSharp10);
    let def = Container::<String>::csharp_definition(&cfg);
    assert!(
        def.contains("uint Count"),
        "concrete field should be uint:\n{def}"
    );
}

#[test]
fn generic_struct_two_params_definition() {
    let cfg = Config::default().with_target(CSharpVersion::CSharp10);
    let def = Pair::<String, i32>::csharp_definition(&cfg);
    assert!(def.contains("Pair<A, B>"), "should have Pair<A, B>:\n{def}");
    assert!(def.contains("A First"), "first field should be A:\n{def}");
    assert!(def.contains("B Second"), "second field should be B:\n{def}");
}

// -----------------------------------------------------------------------
// Nested generics
// -----------------------------------------------------------------------

#[derive(CSharp)]
struct Wrapper<T> {
    items: Vec<T>,
    lookup: std::collections::HashMap<String, T>,
    maybe: Option<T>,
}

#[test]
fn nested_generic_vec_field() {
    let cfg = Config::default().with_target(CSharpVersion::CSharp10);
    let def = Wrapper::<String>::csharp_definition(&cfg);
    assert!(
        def.contains("List<T> Items"),
        "Vec<T> should become List<T>:\n{def}"
    );
}

#[test]
fn nested_generic_hashmap_field() {
    let cfg = Config::default().with_target(CSharpVersion::CSharp10);
    let def = Wrapper::<String>::csharp_definition(&cfg);
    assert!(
        def.contains("Dictionary<string, T> Lookup"),
        "HashMap<String, T> should become Dictionary<string, T>:\n{def}"
    );
}

#[test]
fn nested_generic_option_field() {
    let cfg = Config::default().with_target(CSharpVersion::CSharp10);
    let def = Wrapper::<String>::csharp_definition(&cfg);
    assert!(
        def.contains("T? Maybe"),
        "Option<T> should become T?:\n{def}"
    );
}

// -----------------------------------------------------------------------
// Generic struct + serde attributes
// -----------------------------------------------------------------------

#[derive(CSharp)]
#[serde(rename_all = "camelCase")]
struct GameState<T> {
    current_round: u32,
    active_player: T,
    #[serde(skip)]
    _internal: String,
}

#[test]
fn generic_with_rename_all() {
    let cfg = Config::default().with_target(CSharpVersion::CSharp10);
    let def = GameState::<String>::csharp_definition(&cfg);
    assert!(
        def.contains("\"currentRound\""),
        "rename_all should apply:\n{def}"
    );
    assert!(
        def.contains("\"activePlayer\""),
        "rename_all should apply to generic field:\n{def}"
    );
}

#[test]
fn generic_with_skip() {
    let cfg = Config::default().with_target(CSharpVersion::CSharp10);
    let def = GameState::<String>::csharp_definition(&cfg);
    assert!(
        !def.contains("Internal"),
        "skipped field should not appear:\n{def}"
    );
}

#[derive(CSharp)]
struct WithRename<T> {
    #[serde(rename = "custom_name")]
    value: T,
}

#[test]
fn generic_with_field_rename() {
    let cfg = Config::default().with_target(CSharpVersion::CSharp10);
    let def = WithRename::<String>::csharp_definition(&cfg);
    assert!(
        def.contains("\"custom_name\""),
        "field rename should apply:\n{def}"
    );
    assert!(
        def.contains("T Value"),
        "field type should still be generic T:\n{def}"
    );
}

// -----------------------------------------------------------------------
// Lifetime params are not tested here because `&'a str` does not implement
// CSharp. Lifetime support is a Rust-side concern only (preserved in the
// impl header, ignored in C# output).
// -----------------------------------------------------------------------

// -----------------------------------------------------------------------
// Multi-config: serializer
// -----------------------------------------------------------------------

#[test]
fn generic_struct_newtonsoft() {
    let cfg = Config::default()
        .with_serializer(Serializer::Newtonsoft)
        .with_target(CSharpVersion::CSharp10);
    let def = Container::<String>::csharp_definition(&cfg);
    assert!(
        def.contains("Container<T>"),
        "Newtonsoft should still have generic name:\n{def}"
    );
    assert!(
        def.contains("[JsonProperty("),
        "Newtonsoft should use JsonProperty:\n{def}"
    );
    assert!(
        def.contains("using Newtonsoft.Json;"),
        "Newtonsoft should have Newtonsoft using:\n{def}"
    );
}

// -----------------------------------------------------------------------
// Multi-config: C# version
// -----------------------------------------------------------------------

#[test]
fn generic_struct_unity() {
    let cfg = Config::default().with_target(CSharpVersion::Unity);
    let def = Container::<String>::csharp_definition(&cfg);
    assert!(
        def.contains("sealed class Container<T>"),
        "Unity should use sealed class:\n{def}"
    );
    assert!(
        def.contains("{ get; set; }"),
        "Unity should use get/set:\n{def}"
    );
}

#[test]
fn generic_struct_csharp9_block_scoped_ns() {
    let cfg = Config::default().with_target(CSharpVersion::CSharp9);
    let def = Container::<String>::csharp_definition(&cfg);
    assert!(
        def.contains("namespace Generated\n{"),
        "C# 9 should use block-scoped namespace:\n{def}"
    );
    assert!(
        def.contains("Container<T>"),
        "should still have generic name:\n{def}"
    );
}

#[test]
fn generic_struct_csharp11_required() {
    let cfg = Config::default().with_target(CSharpVersion::CSharp11);
    let def = Container::<String>::csharp_definition(&cfg);
    assert!(
        def.contains("required T Item"),
        "C# 11 should have required on non-optional generic field:\n{def}"
    );
    assert!(
        def.contains("required uint Count"),
        "C# 11 should have required on concrete field:\n{def}"
    );
}

// -----------------------------------------------------------------------
// Dependencies
// -----------------------------------------------------------------------

#[test]
fn generic_struct_dependencies() {
    let cfg = Config::default();
    let deps = Container::<String>::dependencies(&cfg);
    assert!(
        deps.contains(&String::from("string")),
        "should depend on concrete type string: {deps:?}"
    );
    assert!(
        deps.contains(&String::from("uint")),
        "should depend on uint: {deps:?}"
    );
}

#[test]
fn generic_struct_name_propagates_in_dependencies() {
    // When Container<T> is used as a field in another struct, its name should
    // resolve with concrete types.
    let cfg = Config::default();
    let name = Container::<i32>::csharp_name(&cfg);
    assert_eq!(name, "Container<int>");
}

// -----------------------------------------------------------------------
// Generic tagged enums
// -----------------------------------------------------------------------

#[derive(CSharp)]
#[serde(tag = "type")]
enum Response<T> {
    Ok { data: T },
    Error { message: String },
}

#[test]
fn generic_tagged_enum_name() {
    let cfg = Config::default();
    assert_eq!(Response::<String>::csharp_name(&cfg), "Response<string>");
}

#[test]
fn generic_tagged_enum_definition_has_generic_abstract_record() {
    let cfg = Config::default().with_target(CSharpVersion::CSharp10);
    let def = Response::<String>::csharp_definition(&cfg);
    assert!(
        def.contains("abstract record Response<T>"),
        "missing generic abstract record:\n{def}"
    );
}

#[test]
fn generic_tagged_enum_variant_inherits_base_generic() {
    let cfg = Config::default().with_target(CSharpVersion::CSharp10);
    let def = Response::<String>::csharp_definition(&cfg);
    assert!(
        def.contains(": Response<T>"),
        "variant should inherit Response<T>:\n{def}"
    );
}

#[test]
fn generic_tagged_enum_has_converter_factory_stj() {
    let cfg = Config::default().with_target(CSharpVersion::CSharp10);
    let def = Response::<String>::csharp_definition(&cfg);
    assert!(
        def.contains("ConverterFactory"),
        "STJ generic should use factory:\n{def}"
    );
}

#[test]
fn generic_tagged_enum_no_native_polymorphism_stj_csharp11() {
    // C# 11 + STJ + internal tag would normally use native polymorphism,
    // but generic types cannot use typeof(Response<T>.Ok) so it must be gated.
    let cfg = Config::default().with_target(CSharpVersion::CSharp11);
    let def = Response::<String>::csharp_definition(&cfg);
    assert!(
        !def.contains("[JsonPolymorphic"),
        "generic enums must not use native polymorphism:\n{def}"
    );
    assert!(
        def.contains("ConverterFactory"),
        "generic enums must use factory:\n{def}"
    );
}

#[test]
fn generic_tagged_enum_newtonsoft_non_generic_converter() {
    let cfg = Config::default()
        .with_target(CSharpVersion::CSharp10)
        .with_serializer(Serializer::Newtonsoft);
    let def = Response::<String>::csharp_definition(&cfg);
    assert!(
        def.contains("ResponseConverter : JsonConverter"),
        "Newtonsoft should have non-generic converter:\n{def}"
    );
    assert!(
        !def.contains("ConverterFactory"),
        "Newtonsoft should not use factory:\n{def}"
    );
    assert!(
        def.contains("CanConvert"),
        "Newtonsoft generic converter should have CanConvert:\n{def}"
    );
}

#[test]
fn generic_tagged_enum_converter_has_generic_type_params() {
    let cfg = Config::default().with_target(CSharpVersion::CSharp10);
    let def = Response::<String>::csharp_definition(&cfg);
    assert!(
        def.contains("ResponseConverter<T>"),
        "STJ converter should be generic:\n{def}"
    );
    assert!(
        def.contains("JsonConverter<Response<T>>"),
        "STJ converter should extend JsonConverter<Response<T>>:\n{def}"
    );
}

#[test]
fn generic_tagged_enum_ok_variant_has_generic_field() {
    let cfg = Config::default().with_target(CSharpVersion::CSharp10);
    let def = Response::<String>::csharp_definition(&cfg);
    assert!(
        def.contains("T Data"),
        "Ok variant should have generic T field:\n{def}"
    );
}

#[test]
fn generic_tagged_enum_error_variant_has_concrete_field() {
    let cfg = Config::default().with_target(CSharpVersion::CSharp10);
    let def = Response::<String>::csharp_definition(&cfg);
    assert!(
        def.contains("string Message"),
        "Error variant should have concrete string field:\n{def}"
    );
}

// -----------------------------------------------------------------------
// Generic tagged enum — Unity (sealed class)
// -----------------------------------------------------------------------

#[test]
fn generic_tagged_enum_unity() {
    let cfg = Config::default().with_target(CSharpVersion::Unity);
    let def = Response::<String>::csharp_definition(&cfg);
    assert!(
        def.contains("abstract class Response<T>"),
        "Unity should use abstract class:\n{def}"
    );
    assert!(
        def.contains(": Response<T>"),
        "Unity variants should inherit generic base:\n{def}"
    );
}

// -----------------------------------------------------------------------
// Non-generic types still work (regression)
// -----------------------------------------------------------------------

#[derive(CSharp)]
struct Plain {
    name: String,
    level: i32,
}

#[test]
fn non_generic_struct_still_works() {
    let cfg = Config::default().with_target(CSharpVersion::CSharp10);
    let def = Plain::csharp_definition(&cfg);
    assert!(
        def.contains("sealed record Plain"),
        "non-generic should work:\n{def}"
    );
    assert!(
        !def.contains("Plain<"),
        "non-generic should not have generic params:\n{def}"
    );
}

#[derive(CSharp)]
#[serde(tag = "kind")]
enum Action {
    Move { x: f64, y: f64 },
    Stop,
}

#[test]
fn non_generic_tagged_enum_still_works() {
    let cfg = Config::default().with_target(CSharpVersion::CSharp10);
    let def = Action::csharp_definition(&cfg);
    assert!(
        def.contains("abstract record Action"),
        "non-generic enum should work:\n{def}"
    );
}

#[test]
fn non_generic_tagged_enum_csharp11_uses_native_polymorphism() {
    let cfg = Config::default().with_target(CSharpVersion::CSharp11);
    let def = Action::csharp_definition(&cfg);
    assert!(
        def.contains("[JsonPolymorphic"),
        "non-generic C# 11 should still use native polymorphism:\n{def}"
    );
}

// -----------------------------------------------------------------------
// Generic struct used as field type in another struct
// -----------------------------------------------------------------------

#[derive(CSharp)]
struct Outer {
    inner: Container<String>,
    pair: Pair<i32, bool>,
}

#[test]
fn generic_type_as_field_resolves_name() {
    let cfg = Config::default().with_target(CSharpVersion::CSharp10);
    let def = Outer::csharp_definition(&cfg);
    assert!(
        def.contains("Container<string> Inner"),
        "Container<String> should resolve to Container<string>:\n{def}"
    );
    assert!(
        def.contains("Pair<int, bool> Pair"),
        "Pair<i32, bool> should resolve to Pair<int, bool>:\n{def}"
    );
}

// -----------------------------------------------------------------------
// Three type params
// -----------------------------------------------------------------------

#[derive(CSharp)]
struct Triple<X, Y, Z> {
    x: X,
    y: Y,
    z: Z,
}

#[test]
fn three_type_params_name() {
    let cfg = Config::default();
    assert_eq!(
        Triple::<String, i32, bool>::csharp_name(&cfg),
        "Triple<string, int, bool>"
    );
}

#[test]
fn three_type_params_definition() {
    let cfg = Config::default().with_target(CSharpVersion::CSharp10);
    let def = Triple::<String, i32, bool>::csharp_definition(&cfg);
    assert!(
        def.contains("Triple<X, Y, Z>"),
        "should have Triple<X, Y, Z>:\n{def}"
    );
}

// -----------------------------------------------------------------------
// #[csharp(concrete(...))] attribute
// -----------------------------------------------------------------------

#[derive(CSharp)]
#[csharp(concrete(M = String))]
struct WithPhantom<T, M> {
    value: T,
    #[serde(skip)]
    _marker: std::marker::PhantomData<M>,
}

#[test]
fn concrete_excludes_param_from_csharp_name() {
    let cfg = Config::default();
    let name = WithPhantom::<i32, String>::csharp_name(&cfg);
    assert_eq!(name, "WithPhantom<int>", "M should be excluded");
}

#[test]
fn concrete_excludes_param_from_definition() {
    let cfg = Config::default().with_target(CSharpVersion::CSharp10);
    let def = WithPhantom::<i32, String>::csharp_definition(&cfg);
    assert!(
        def.contains("WithPhantom<T>"),
        "definition should only have T, not M:\n{def}"
    );
    assert!(
        !def.contains("WithPhantom<T, M>"),
        "M should not appear in generic params:\n{def}"
    );
}

#[test]
fn concrete_field_uses_generic_type() {
    let cfg = Config::default().with_target(CSharpVersion::CSharp10);
    let def = WithPhantom::<i32, String>::csharp_definition(&cfg);
    assert!(
        def.contains("T Value"),
        "non-concrete field should use T:\n{def}"
    );
}

// -----------------------------------------------------------------------
// #[csharp(bound = "...")] attribute
// -----------------------------------------------------------------------

#[derive(CSharp)]
#[csharp(bound = "T: csharp_rs::CSharp")]
struct WithBound<T> {
    value: T,
}

#[test]
fn custom_bound_compiles_and_generates() {
    let cfg = Config::default().with_target(CSharpVersion::CSharp10);
    let def = WithBound::<String>::csharp_definition(&cfg);
    assert!(
        def.contains("WithBound<T>"),
        "custom bound should still produce generic output:\n{def}"
    );
    assert!(
        def.contains("T Value"),
        "field should use generic T:\n{def}"
    );
}
