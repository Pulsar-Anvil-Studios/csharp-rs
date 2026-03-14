// Rust guideline compliant 2026-02-11
//! Integration tests for `#[derive(CSharp)]` on tagged enums.

#![expect(dead_code, reason = "test enums are only used via derive macro")]

use csharp_rs::{CSharp, CSharpVersion, Config, Serializer};

// --- internally tagged: struct + unit variants ---

#[derive(CSharp)]
#[serde(tag = "type")]
enum Message {
    Request { id: String, method: String },
    Quit,
}

#[test]
fn internally_tagged_has_abstract_record() {
    let cfg = Config::default();
    let def = Message::csharp_definition(&cfg);
    assert!(
        def.contains("public abstract record Message"),
        "missing abstract record:\n{def}"
    );
}

#[test]
fn internally_tagged_has_derived_records() {
    let cfg = Config::default();
    let def = Message::csharp_definition(&cfg);
    assert!(
        def.contains("sealed record Request : Message"),
        "missing sealed record Request:\n{def}"
    );
    assert!(
        def.contains("sealed record Quit : Message"),
        "missing sealed record Quit:\n{def}"
    );
}

#[test]
fn internally_tagged_has_converter() {
    let cfg = Config::default();
    let def = Message::csharp_definition(&cfg);
    assert!(
        def.contains("MessageConverter"),
        "missing MessageConverter (C# 9 uses converter path):\n{def}"
    );
    assert!(
        def.contains("[JsonConverter(typeof(MessageConverter))]"),
        "missing [JsonConverter] attribute:\n{def}"
    );
}

#[test]
fn internally_tagged_request_has_properties() {
    let cfg = Config::default();
    let def = Message::csharp_definition(&cfg);
    assert!(
        def.contains("[JsonPropertyName(\"id\")]"),
        "missing JsonPropertyName for id:\n{def}"
    );
    assert!(
        def.contains("public string Id { get; init; }"),
        "missing Id property:\n{def}"
    );
    assert!(
        def.contains("[JsonPropertyName(\"method\")]"),
        "missing JsonPropertyName for method:\n{def}"
    );
    assert!(
        def.contains("public string Method { get; init; }"),
        "missing Method property:\n{def}"
    );
}

// --- internally tagged: newtype + unit ---

#[derive(CSharp)]
#[serde(tag = "kind")]
enum Value {
    Text(String),
    Number(f64),
    Null,
}

#[test]
fn internally_tagged_newtype_has_value_property() {
    let cfg = Config::default();
    let def = Value::csharp_definition(&cfg);
    assert!(
        def.contains("public string Value { get; init; }"),
        "Text variant should have string Value property:\n{def}"
    );
    assert!(
        def.contains("public double Value { get; init; }"),
        "Number variant should have double Value property:\n{def}"
    );
}

#[test]
fn internally_tagged_unit_has_no_properties() {
    let cfg = Config::default();
    let def = Value::csharp_definition(&cfg);
    assert!(
        def.contains("sealed record Null : Value;"),
        "Null should be a semicolon record with no body:\n{def}"
    );
}

// --- internally tagged: with rename_all ---

#[derive(CSharp)]
#[serde(tag = "type", rename_all = "camelCase")]
enum Event {
    UserLogin { user_id: String },
    SessionEnd,
}

#[test]
fn internally_tagged_rename_all_variant_names() {
    let cfg = Config::default();
    let def = Event::csharp_definition(&cfg);
    // Discriminator values (JSON names) should be camelCase.
    assert!(
        def.contains("\"userLogin\""),
        "UserLogin discriminator should be camelCase:\n{def}"
    );
    assert!(
        def.contains("\"sessionEnd\""),
        "SessionEnd discriminator should be camelCase:\n{def}"
    );
}

// --- externally tagged: struct + newtype + unit ---

#[derive(CSharp)]
enum Shape {
    Circle { radius: f64 },
    Point(String),
    Unknown,
}

#[test]
fn externally_tagged_has_abstract_record() {
    let cfg = Config::default();
    let def = Shape::csharp_definition(&cfg);
    assert!(
        def.contains("public abstract record Shape"),
        "missing abstract record Shape:\n{def}"
    );
}

#[test]
fn externally_tagged_has_converter() {
    let cfg = Config::default();
    let def = Shape::csharp_definition(&cfg);
    assert!(
        def.contains("ShapeConverter"),
        "missing ShapeConverter:\n{def}"
    );
    assert!(
        def.contains("[JsonConverter(typeof(ShapeConverter))]"),
        "missing [JsonConverter] attribute:\n{def}"
    );
}

#[test]
fn externally_tagged_has_variant_records() {
    let cfg = Config::default();
    let def = Shape::csharp_definition(&cfg);
    assert!(
        def.contains("sealed record Circle : Shape"),
        "missing Circle variant record:\n{def}"
    );
    assert!(
        def.contains("sealed record Point : Shape"),
        "missing Point variant record:\n{def}"
    );
    assert!(
        def.contains("sealed record Unknown : Shape"),
        "missing Unknown variant record:\n{def}"
    );
}

// --- adjacently tagged: struct + newtype + unit ---

#[derive(CSharp)]
#[serde(tag = "t", content = "c")]
enum Block {
    Paragraph { text: String },
    Code(String),
    Break,
}

#[test]
fn adjacently_tagged_has_converter() {
    let cfg = Config::default();
    let def = Block::csharp_definition(&cfg);
    assert!(
        def.contains("BlockConverter"),
        "missing BlockConverter:\n{def}"
    );
    assert!(
        def.contains("[JsonConverter(typeof(BlockConverter))]"),
        "missing [JsonConverter] attribute:\n{def}"
    );
}

#[test]
fn adjacently_tagged_has_abstract_record() {
    let cfg = Config::default();
    let def = Block::csharp_definition(&cfg);
    assert!(
        def.contains("public abstract record Block"),
        "missing abstract record Block:\n{def}"
    );
}

// --- untagged: struct + newtype ---

#[derive(CSharp)]
#[serde(untagged)]
enum Data {
    Text(String),
    Object { key: String, value: String },
}

#[test]
fn untagged_has_converter() {
    let cfg = Config::default();
    let def = Data::csharp_definition(&cfg);
    assert!(
        def.contains("DataConverter"),
        "missing DataConverter:\n{def}"
    );
    assert!(
        def.contains("[JsonConverter(typeof(DataConverter))]"),
        "missing [JsonConverter] attribute:\n{def}"
    );
}

#[test]
fn untagged_has_abstract_record() {
    let cfg = Config::default();
    let def = Data::csharp_definition(&cfg);
    assert!(
        def.contains("public abstract record Data"),
        "missing abstract record Data:\n{def}"
    );
}

// --- namespace override ---

#[derive(CSharp)]
#[serde(tag = "type")]
#[csharp(namespace = "Game.Network")]
enum Packet {
    Ping,
    Data { payload: Vec<u8> },
}

#[test]
fn tagged_namespace_override() {
    let cfg = Config::default();
    let def = Packet::csharp_definition(&cfg);
    assert!(
        def.contains("namespace Game.Network"),
        "missing namespace override:\n{def}"
    );
}

// --- dependencies include variant field types ---

#[test]
fn tagged_enum_dependencies() {
    let cfg = Config::default();
    let deps = Message::dependencies(&cfg);
    // Message has Request { id: String, method: String } and Quit (unit).
    // Dependencies should include "string" from the struct variant fields.
    assert!(
        deps.contains(&String::from("string")),
        "dependencies should contain 'string' from Request fields: {deps:?}"
    );
}

// --- per-variant rename + skip ---

#[derive(CSharp)]
#[serde(tag = "type")]
enum Action {
    #[serde(rename = "CLICK")]
    Click {
        x: i32,
        y: i32,
    },
    #[serde(skip)]
    Internal,
    Move {
        dx: i32,
    },
}

#[test]
fn tagged_per_variant_rename() {
    let cfg = Config::default();
    let def = Action::csharp_definition(&cfg);
    // The discriminator value for Click should be "CLICK" (from serde(rename)).
    assert!(
        def.contains("\"CLICK\""),
        "Click discriminator should be renamed to CLICK:\n{def}"
    );
}

#[test]
fn tagged_skip_variant() {
    let cfg = Config::default();
    let def = Action::csharp_definition(&cfg);
    assert!(
        !def.contains("Internal"),
        "Internal variant should be skipped:\n{def}"
    );
    // Other variants should still be present.
    assert!(
        def.contains("sealed record Click : Action"),
        "Click should be present:\n{def}"
    );
    assert!(
        def.contains("sealed record Move : Action"),
        "Move should be present:\n{def}"
    );
}

// --- auto-generated comment and using directives ---

#[test]
fn tagged_enum_has_auto_generated_comment() {
    let cfg = Config::default();
    let def = Message::csharp_definition(&cfg);
    assert!(
        def.starts_with("// <auto-generated/>"),
        "missing auto-generated comment:\n{def}"
    );
}

#[test]
fn tagged_enum_has_using_directives() {
    let cfg = Config::default();
    let def = Message::csharp_definition(&cfg);
    // C# 9 + STJ converter path requires System, System.Text.Json, and
    // System.Text.Json.Serialization.
    assert!(
        def.contains("using System;"),
        "missing System using:\n{def}"
    );
    assert!(
        def.contains("using System.Text.Json;"),
        "missing System.Text.Json using:\n{def}"
    );
    assert!(
        def.contains("using System.Text.Json.Serialization;"),
        "missing System.Text.Json.Serialization using:\n{def}"
    );
}

#[test]
fn tagged_enum_default_namespace() {
    let cfg = Config::default();
    let def = Message::csharp_definition(&cfg);
    assert!(
        def.contains("namespace Generated"),
        "missing default namespace:\n{def}"
    );
}

// --- rename_all applied to struct variant field names ---

#[test]
fn internally_tagged_rename_all_field_names() {
    let cfg = Config::default();
    let def = Event::csharp_definition(&cfg);
    // user_id with camelCase rename_all should have JSON name "userId".
    assert!(
        def.contains("[JsonPropertyName(\"userId\")]"),
        "user_id field should be renamed to camelCase 'userId':\n{def}"
    );
    // C# property should be PascalCase.
    assert!(
        def.contains("public string UserId { get; init; }"),
        "C# property should be PascalCase UserId:\n{def}"
    );
}

// --- externally tagged variant with properties ---

#[test]
fn externally_tagged_circle_has_radius_property() {
    let cfg = Config::default();
    let def = Shape::csharp_definition(&cfg);
    assert!(
        def.contains("[JsonPropertyName(\"radius\")]"),
        "Circle should have JsonPropertyName for radius:\n{def}"
    );
    assert!(
        def.contains("public double Radius { get; init; }"),
        "Circle should have double Radius property:\n{def}"
    );
}

#[test]
fn externally_tagged_point_has_value_property() {
    let cfg = Config::default();
    let def = Shape::csharp_definition(&cfg);
    assert!(
        def.contains("sealed record Point : Shape"),
        "Point should be a sealed record:\n{def}"
    );
    // Newtype variant has a "Value" property.
    assert!(
        def.contains("public string Value { get; init; }"),
        "Point (newtype) should have string Value property:\n{def}"
    );
}

// --- adjacently tagged variant details ---

#[test]
fn adjacently_tagged_paragraph_has_text_property() {
    let cfg = Config::default();
    let def = Block::csharp_definition(&cfg);
    assert!(
        def.contains("[JsonPropertyName(\"text\")]"),
        "Paragraph should have JsonPropertyName for text:\n{def}"
    );
    assert!(
        def.contains("public string Text { get; init; }"),
        "Paragraph should have string Text property:\n{def}"
    );
}

#[test]
fn adjacently_tagged_unit_has_no_body() {
    let cfg = Config::default();
    let def = Block::csharp_definition(&cfg);
    assert!(
        def.contains("sealed record Break : Block;"),
        "Break should be a semicolon record with no body:\n{def}"
    );
}

// --- untagged variant details ---

#[test]
fn untagged_has_variant_records() {
    let cfg = Config::default();
    let def = Data::csharp_definition(&cfg);
    assert!(
        def.contains("sealed record Text : Data"),
        "missing Text variant record:\n{def}"
    );
    assert!(
        def.contains("sealed record Object : Data"),
        "missing Object variant record:\n{def}"
    );
}

#[test]
fn untagged_object_has_properties() {
    let cfg = Config::default();
    let def = Data::csharp_definition(&cfg);
    assert!(
        def.contains("[JsonPropertyName(\"key\")]"),
        "Object should have JsonPropertyName for key:\n{def}"
    );
    assert!(
        def.contains("public string Key { get; init; }"),
        "Object should have string Key property:\n{def}"
    );
    assert!(
        def.contains("[JsonPropertyName(\"value\")]"),
        "Object should have JsonPropertyName for value:\n{def}"
    );
    assert!(
        def.contains("public string Value { get; init; }"),
        "Object should have string Value property:\n{def}"
    );
}

// --- multi-config: serializer switching ---

#[test]
fn tagged_enum_newtonsoft_has_newtonsoft_converter() {
    let cfg = Config::default().with_serializer(Serializer::Newtonsoft);
    let def = Message::csharp_definition(&cfg);
    assert!(
        def.contains("using Newtonsoft.Json;"),
        "Newtonsoft tagged enum should have Newtonsoft.Json using:\n{def}"
    );
    assert!(
        def.contains("JObject.Load"),
        "Newtonsoft converter should use JObject.Load:\n{def}"
    );
}

#[test]
fn tagged_enum_stj_csharp9_uses_converter() {
    let cfg = Config::default();
    let def = Message::csharp_definition(&cfg);
    assert!(
        def.contains("MessageConverter"),
        "C# 9 + STJ should use converter path:\n{def}"
    );
    assert!(
        def.contains("JsonDocument.ParseValue"),
        "C# 9 + STJ converter should use JsonDocument:\n{def}"
    );
}

// --- multi-config: C# 11 native polymorphism ---

#[test]
fn tagged_enum_csharp11_stj_uses_native_polymorphism() {
    let cfg = Config::default().with_target(CSharpVersion::CSharp11);
    let def = Message::csharp_definition(&cfg);
    assert!(
        def.contains("[JsonPolymorphic"),
        "C# 11 + STJ should use native [JsonPolymorphic]:\n{def}"
    );
    assert!(
        def.contains("[JsonDerivedType"),
        "C# 11 + STJ should use [JsonDerivedType]:\n{def}"
    );
    assert!(
        !def.contains("MessageConverter"),
        "C# 11 + STJ should NOT generate converter class:\n{def}"
    );
}

#[test]
fn tagged_enum_csharp11_newtonsoft_still_uses_converter() {
    let cfg = Config::default()
        .with_serializer(Serializer::Newtonsoft)
        .with_target(CSharpVersion::CSharp11);
    let def = Message::csharp_definition(&cfg);
    assert!(
        def.contains("MessageConverter"),
        "C# 11 + Newtonsoft should still use converter:\n{def}"
    );
    assert!(
        !def.contains("JsonPolymorphic"),
        "Newtonsoft should NOT use [JsonPolymorphic]:\n{def}"
    );
}

// --- multi-config: C# version switching ---

#[test]
fn tagged_enum_csharp10_file_scoped_namespace() {
    let cfg = Config::default().with_target(CSharpVersion::CSharp10);
    let def = Message::csharp_definition(&cfg);
    assert!(
        def.contains("namespace Generated;"),
        "C# 10 tagged enum should use file-scoped namespace:\n{def}"
    );
}

#[test]
fn tagged_enum_csharp11_has_required_modifier() {
    let cfg = Config::default().with_target(CSharpVersion::CSharp11);
    let def = Message::csharp_definition(&cfg);
    assert!(
        def.contains("public required string Id"),
        "C# 11 tagged enum struct variant should have required modifier:\n{def}"
    );
}

// --- rename_all_fields applies to variant struct fields independently ---

#[derive(CSharp)]
#[serde(tag = "type", rename_all = "UPPERCASE", rename_all_fields = "camelCase")]
enum ApiEvent {
    UserLogin {
        user_name: String,
        login_time: String,
    },
}

#[test]
fn rename_all_fields_applies_to_variant_fields_independently() {
    let cfg = Config::default();
    let def = ApiEvent::csharp_definition(&cfg);
    // Variant discriminator should use rename_all (UPPERCASE): "USERLOGIN"
    // UPPERCASE joins words without separator per serde convention.
    assert!(
        def.contains("\"USERLOGIN\""),
        "variant discriminator should use rename_all UPPERCASE:\n{def}"
    );
    // Field names should use rename_all_fields (camelCase): "userName", "loginTime"
    assert!(
        def.contains("\"userName\""),
        "field should use rename_all_fields camelCase:\n{def}"
    );
    assert!(
        def.contains("\"loginTime\""),
        "field should use rename_all_fields camelCase:\n{def}"
    );
}

#[test]
fn rename_all_fields_does_not_affect_variant_names() {
    let cfg = Config::default();
    let def = ApiEvent::csharp_definition(&cfg);
    // C# type name stays PascalCase regardless.
    assert!(
        def.contains("sealed record UserLogin : ApiEvent"),
        "C# variant name should remain PascalCase:\n{def}"
    );
}
