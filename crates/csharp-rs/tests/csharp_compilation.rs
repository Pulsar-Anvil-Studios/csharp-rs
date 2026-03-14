// Rust guideline compliant 2026-03-14
//! Integration tests that verify generated C# code actually compiles with `dotnet build`.
//!
//! Each test exports all derive types for a specific C# version + serializer combination,
//! writes a `.csproj`, and runs `dotnet build`. Tests are `#[ignore]` because they require
//! a .NET SDK installed on the machine.

#![expect(dead_code, reason = "test types are only used via derive macro")]

use std::collections::HashMap;

use csharp_rs::{CSharp, CSharpVersion, Config, Serializer};

// ---------------------------------------------------------------------------
// Struct features
// ---------------------------------------------------------------------------

#[derive(CSharp)]
struct BasicStruct {
    name: String,
    level: i32,
    score: f64,
    active: bool,
}

#[derive(CSharp)]
struct WithOptionals {
    required_name: String,
    optional_tag: Option<String>,
    optional_count: Option<i32>,
}

#[derive(CSharp)]
struct WithCollections {
    items: Vec<String>,
    scores: Vec<f64>,
    lookup: HashMap<String, i32>,
}

#[derive(CSharp)]
#[serde(rename_all = "camelCase")]
struct WithRenameAll {
    player_name: String,
    high_score: i64,
}

#[derive(CSharp)]
struct WithFieldAttrs {
    #[serde(rename = "id")]
    player_id: String,
    #[serde(skip)]
    internal_state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    nickname: Option<String>,
    #[serde(default)]
    retry_count: i32,
    #[csharp(type = "object")]
    dynamic_data: String,
}

#[derive(CSharp)]
#[csharp(namespace = "Game.Models")]
struct WithNamespace {
    value: String,
}

#[derive(CSharp)]
struct FlattenBase {
    page: u32,
    per_page: u32,
}

#[derive(CSharp)]
struct WithFlatten {
    name: String,
    #[serde(flatten)]
    pagination: FlattenBase,
    #[serde(flatten)]
    extra: HashMap<String, String>,
}

// ---------------------------------------------------------------------------
// Newtype features
// ---------------------------------------------------------------------------

#[derive(CSharp)]
struct UserId(String);

#[derive(CSharp)]
#[serde(transparent)]
struct PlayerId(String);

// ---------------------------------------------------------------------------
// Simple enum features
// ---------------------------------------------------------------------------

#[derive(CSharp)]
enum Color {
    Red,
    Green,
    Blue,
}

#[derive(CSharp)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum Status {
    Active,
    #[serde(rename = "DISABLED")]
    Inactive,
    #[serde(skip)]
    Hidden,
}

// ---------------------------------------------------------------------------
// Tagged enum features (all 4 serde modes)
// ---------------------------------------------------------------------------

#[derive(CSharp)]
#[serde(tag = "type")]
enum InternallyTagged {
    Request { id: String, method: String },
    Text(String),
    Quit,
}

#[derive(CSharp)]
enum ExternallyTagged {
    Circle { radius: f64 },
    Point(String),
    Empty,
}

#[derive(CSharp)]
#[serde(tag = "t", content = "c")]
enum AdjacentlyTagged {
    Data { key: String, value: i32 },
    Label(String),
    None,
}

#[derive(CSharp)]
#[serde(untagged)]
enum Untagged {
    Obj { key: String, value: String },
    Text(String),
    Nothing,
}

#[derive(CSharp)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
enum WithEnumRenaming {
    UserLogin {
        user_name: String,
        login_time: String,
    },
}

// ---------------------------------------------------------------------------
// Compilation helper
// ---------------------------------------------------------------------------

/// Exports all test types as `.cs` files, writes a `.csproj`, and runs `dotnet build`.
///
/// Panics if `dotnet` is not found or the build fails.
fn compile_all_types(
    version: CSharpVersion,
    serializer: Serializer,
    lang_version: &str,
    target_framework: &str,
) {
    // Verify dotnet is available.
    let dotnet_check = std::process::Command::new("dotnet")
        .arg("--version")
        .output();

    match dotnet_check {
        Ok(output) if output.status.success() => {}
        _ => panic!(
            "dotnet SDK not found. Install .NET SDK from https://dotnet.microsoft.com/download \
             to run C# compilation tests."
        ),
    }

    let cfg = Config::default()
        .with_serializer(serializer)
        .with_target(version);

    // Use version enum, lang_version, serializer, and process ID for a unique temp directory.
    let dir = std::env::temp_dir().join(format!(
        "csharp_rs_compile_test_{version:?}_{lang_version}_{serializer:?}_{}",
        std::process::id()
    ));

    // Clean and create the temp directory.
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");

    // Export all types.
    let types: Vec<(&str, String)> = vec![
        ("BasicStruct", BasicStruct::csharp_definition(&cfg)),
        ("WithOptionals", WithOptionals::csharp_definition(&cfg)),
        ("WithCollections", WithCollections::csharp_definition(&cfg)),
        ("WithRenameAll", WithRenameAll::csharp_definition(&cfg)),
        ("WithFieldAttrs", WithFieldAttrs::csharp_definition(&cfg)),
        ("WithNamespace", WithNamespace::csharp_definition(&cfg)),
        ("FlattenBase", FlattenBase::csharp_definition(&cfg)),
        ("WithFlatten", WithFlatten::csharp_definition(&cfg)),
        ("UserId", UserId::csharp_definition(&cfg)),
        ("PlayerId", PlayerId::csharp_definition(&cfg)),
        ("Color", Color::csharp_definition(&cfg)),
        ("Status", Status::csharp_definition(&cfg)),
        (
            "InternallyTagged",
            InternallyTagged::csharp_definition(&cfg),
        ),
        (
            "ExternallyTagged",
            ExternallyTagged::csharp_definition(&cfg),
        ),
        (
            "AdjacentlyTagged",
            AdjacentlyTagged::csharp_definition(&cfg),
        ),
        ("Untagged", Untagged::csharp_definition(&cfg)),
        (
            "WithEnumRenaming",
            WithEnumRenaming::csharp_definition(&cfg),
        ),
    ];

    for (name, definition) in &types {
        if !definition.is_empty() {
            let file_path = dir.join(format!("{name}.cs"));
            std::fs::write(&file_path, definition)
                .unwrap_or_else(|e| panic!("write {name}.cs: {e}"));
        }
    }

    // Write .csproj file.
    let newtonsoft_pkg = if serializer == Serializer::Newtonsoft {
        "\n    <PackageReference Include=\"Newtonsoft.Json\" Version=\"13.*\" />"
    } else {
        ""
    };

    let csproj = format!(
        r#"<Project Sdk="Microsoft.NET.Sdk">
  <PropertyGroup>
    <TargetFramework>{target_framework}</TargetFramework>
    <LangVersion>{lang_version}</LangVersion>
    <Nullable>enable</Nullable>
    <OutputType>Library</OutputType>
  </PropertyGroup>
  <ItemGroup>{newtonsoft_pkg}
  </ItemGroup>
</Project>"#
    );

    std::fs::write(dir.join("CompileTest.csproj"), csproj).expect("write csproj");

    // Run dotnet build. Suppress first-time setup to avoid race conditions
    // when multiple tests run in parallel.
    let output = std::process::Command::new("dotnet")
        .args(["build", "--nologo", "--verbosity", "quiet"])
        .env("DOTNET_SKIP_FIRST_TIME_EXPERIENCE", "1")
        .env("DOTNET_NOLOGO", "1")
        .env("DOTNET_CLI_TELEMETRY_OPTOUT", "1")
        .current_dir(&dir)
        .output()
        .expect("run dotnet build");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "dotnet build failed for {lang_version}/{serializer:?}:\nSTDOUT:\n{stdout}\nSTDERR:\n{stderr}"
    );

    // Cleanup on success.
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// C# 9 (block-scoped namespaces, no required modifier)
// ---------------------------------------------------------------------------

#[test]
#[ignore = "requires dotnet SDK"]
fn compile_csharp9_stj() {
    compile_all_types(
        CSharpVersion::CSharp9,
        Serializer::SystemTextJson,
        "9.0",
        "net6.0",
    );
}

#[test]
#[ignore = "requires dotnet SDK"]
fn compile_csharp9_newtonsoft() {
    compile_all_types(
        CSharpVersion::CSharp9,
        Serializer::Newtonsoft,
        "9.0",
        "net6.0",
    );
}

// ---------------------------------------------------------------------------
// C# 10 (file-scoped namespaces)
// ---------------------------------------------------------------------------

#[test]
#[ignore = "requires dotnet SDK"]
fn compile_csharp10_stj() {
    compile_all_types(
        CSharpVersion::CSharp10,
        Serializer::SystemTextJson,
        "10.0",
        "net6.0",
    );
}

#[test]
#[ignore = "requires dotnet SDK"]
fn compile_csharp10_newtonsoft() {
    compile_all_types(
        CSharpVersion::CSharp10,
        Serializer::Newtonsoft,
        "10.0",
        "net6.0",
    );
}

// ---------------------------------------------------------------------------
// C# 11 (required modifier, native polymorphism with STJ)
// ---------------------------------------------------------------------------

#[test]
#[ignore = "requires dotnet SDK"]
fn compile_csharp11_stj() {
    compile_all_types(
        CSharpVersion::CSharp11,
        Serializer::SystemTextJson,
        "11.0",
        "net7.0",
    );
}

#[test]
#[ignore = "requires dotnet SDK"]
fn compile_csharp11_newtonsoft() {
    compile_all_types(
        CSharpVersion::CSharp11,
        Serializer::Newtonsoft,
        "11.0",
        "net7.0",
    );
}

// ---------------------------------------------------------------------------
// C# 12 (primary constructors)
// ---------------------------------------------------------------------------

#[test]
#[ignore = "requires dotnet SDK"]
fn compile_csharp12_stj() {
    compile_all_types(
        CSharpVersion::CSharp12,
        Serializer::SystemTextJson,
        "12.0",
        "net8.0",
    );
}

#[test]
#[ignore = "requires dotnet SDK"]
fn compile_csharp12_newtonsoft() {
    compile_all_types(
        CSharpVersion::CSharp12,
        Serializer::Newtonsoft,
        "12.0",
        "net8.0",
    );
}

// ---------------------------------------------------------------------------
// Unity (sealed class + get/set, block-scoped namespaces)
// ---------------------------------------------------------------------------

#[test]
#[ignore = "requires dotnet SDK"]
fn compile_unity_stj() {
    compile_all_types(
        CSharpVersion::Unity,
        Serializer::SystemTextJson,
        "9.0",
        "net6.0",
    );
}

#[test]
#[ignore = "requires dotnet SDK"]
fn compile_unity_newtonsoft() {
    compile_all_types(
        CSharpVersion::Unity,
        Serializer::Newtonsoft,
        "9.0",
        "net6.0",
    );
}

// ---------------------------------------------------------------------------
// Forward compatibility: C# 13 on net9.0
// ---------------------------------------------------------------------------

#[test]
#[ignore = "requires dotnet SDK"]
fn compile_csharp13_forward_compat_stj() {
    compile_all_types(
        CSharpVersion::CSharp12,
        Serializer::SystemTextJson,
        "13.0",
        "net9.0",
    );
}

#[test]
#[ignore = "requires dotnet SDK"]
fn compile_csharp13_forward_compat_newtonsoft() {
    compile_all_types(
        CSharpVersion::CSharp12,
        Serializer::Newtonsoft,
        "13.0",
        "net9.0",
    );
}

// ---------------------------------------------------------------------------
// Forward compatibility: C# 14 on net10.0
// ---------------------------------------------------------------------------

#[test]
#[ignore = "requires dotnet SDK"]
fn compile_csharp14_forward_compat_stj() {
    compile_all_types(
        CSharpVersion::CSharp12,
        Serializer::SystemTextJson,
        "14.0",
        "net10.0",
    );
}

#[test]
#[ignore = "requires dotnet SDK"]
fn compile_csharp14_forward_compat_newtonsoft() {
    compile_all_types(
        CSharpVersion::CSharp12,
        Serializer::Newtonsoft,
        "14.0",
        "net10.0",
    );
}
