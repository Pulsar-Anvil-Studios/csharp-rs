// Rust guideline compliant 2026-03-15
//! Cross-language JSON round-trip E2E tests.
//!
//! Verifies the full cycle:
//! Rust instance -> `serde_json` -> JSON1 -> C# deserialize -> C# re-serialize -> JSON2 -> Rust
//!
//! Each test serializes Rust types, generates a C# xUnit test project,
//! runs `dotnet test`, then deserializes the C#-produced JSON back in Rust.

#![allow(dead_code, reason = "test types used only via derive macros")]

use std::collections::HashMap;

use csharp_rs::{CSharp, CSharpVersion, Config, Serializer};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Test types — dual derive: CSharp + serde
// ---------------------------------------------------------------------------

#[derive(CSharp, Serialize, Deserialize, PartialEq, Debug)]
struct BasicStruct {
    name: String,
    level: i32,
    score: f64,
    active: bool,
}

#[derive(CSharp, Serialize, Deserialize, PartialEq, Debug)]
struct WithOptionals {
    name: String,
    tag: Option<String>,
    count: Option<i32>,
}

#[derive(CSharp, Serialize, Deserialize, PartialEq, Debug)]
struct WithCollections {
    items: Vec<String>,
    lookup: HashMap<String, i32>,
}

#[derive(CSharp, Serialize, Deserialize, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
struct WithRenameAll {
    player_name: String,
    high_score: i64,
}

#[derive(CSharp, Serialize, Deserialize, PartialEq, Debug)]
enum Color {
    Red,
    Green,
    Blue,
}

#[derive(CSharp, Serialize, Deserialize, PartialEq, Debug)]
#[serde(tag = "type")]
enum Message {
    Request { id: String },
    Quit,
}

#[derive(CSharp, Serialize, Deserialize, PartialEq, Debug)]
struct GenericContainer<T> {
    item: T,
    count: u32,
}

#[derive(CSharp, Serialize, Deserialize, PartialEq, Debug)]
#[serde(tag = "type")]
enum GenericResponse<T> {
    Ok { data: T },
    Error { message: String },
}

// ---------------------------------------------------------------------------
// Test data factory
// ---------------------------------------------------------------------------

/// A test case: Rust instance name, JSON, and C# definition.
struct TestCase {
    name: &'static str,
    /// C# type expression for deserialization (e.g., `GenericContainer<string>`).
    csharp_type: &'static str,
    json: String,
    csharp_definition: String,
}

/// Creates all test instances, serializes to JSON, generates C# definitions.
fn build_test_cases(cfg: &Config) -> Vec<TestCase> {
    vec![
        TestCase {
            name: "BasicStruct",
            csharp_type: "BasicStruct",
            json: serde_json::to_string(&BasicStruct {
                name: "Alice".into(),
                level: 42,
                score: 99.5,
                active: true,
            })
            .unwrap(),
            csharp_definition: BasicStruct::csharp_definition(cfg),
        },
        TestCase {
            name: "WithOptionals",
            csharp_type: "WithOptionals",
            json: serde_json::to_string(&WithOptionals {
                name: "Bob".into(),
                tag: Some("vip".into()),
                count: None,
            })
            .unwrap(),
            csharp_definition: WithOptionals::csharp_definition(cfg),
        },
        TestCase {
            name: "WithCollections",
            csharp_type: "WithCollections",
            json: serde_json::to_string(&WithCollections {
                items: vec!["a".into(), "b".into()],
                lookup: [("x".into(), 1), ("y".into(), 2)].into_iter().collect(),
            })
            .unwrap(),
            csharp_definition: WithCollections::csharp_definition(cfg),
        },
        TestCase {
            name: "WithRenameAll",
            csharp_type: "WithRenameAll",
            json: serde_json::to_string(&WithRenameAll {
                player_name: "Charlie".into(),
                high_score: 99999,
            })
            .unwrap(),
            csharp_definition: WithRenameAll::csharp_definition(cfg),
        },
        TestCase {
            name: "Color",
            csharp_type: "Color",
            json: serde_json::to_string(&Color::Green).unwrap(),
            csharp_definition: Color::csharp_definition(cfg),
        },
        TestCase {
            name: "Message",
            csharp_type: "Message",
            json: serde_json::to_string(&Message::Request {
                id: "abc-123".into(),
            })
            .unwrap(),
            csharp_definition: Message::csharp_definition(cfg),
        },
        TestCase {
            name: "GenericContainer",
            csharp_type: "GenericContainer<string>",
            json: serde_json::to_string(&GenericContainer {
                item: "hello".to_string(),
                count: 7,
            })
            .unwrap(),
            csharp_definition: GenericContainer::<String>::csharp_definition(cfg),
        },
        TestCase {
            name: "GenericResponse",
            csharp_type: "GenericResponse<string>",
            json: serde_json::to_string(&GenericResponse::<String>::Ok {
                data: "payload".into(),
            })
            .unwrap(),
            csharp_definition: GenericResponse::<String>::csharp_definition(cfg),
        },
    ]
}

// ---------------------------------------------------------------------------
// C# xUnit project generators
// ---------------------------------------------------------------------------

/// Generates a C# xUnit `[Fact]` method for a single type round-trip.
fn generate_xunit_fact(name: &str, csharp_type: &str, serializer: Serializer) -> String {
    match serializer {
        Serializer::SystemTextJson => format!(
            r#"    [Fact]
    public void {name}_Roundtrip()
    {{
        var json = File.ReadAllText(Path.Combine(BaseDir, "{name}.json"));
        var obj = JsonSerializer.Deserialize<{csharp_type}>(json);
        Assert.NotNull(obj);
        var output = JsonSerializer.Serialize(obj);
        // Structural equality (order-insensitive)
        Assert.Equal(
            JsonDocument.Parse(json).RootElement.ToString(),
            JsonDocument.Parse(output).RootElement.ToString()
        );
        File.WriteAllText(Path.Combine(BaseDir, "{name}.roundtrip.json"), output);
    }}"#
        ),
        Serializer::Newtonsoft => format!(
            r#"    [Fact]
    public void {name}_Roundtrip()
    {{
        var json = File.ReadAllText(Path.Combine(BaseDir, "{name}.json"));
        var obj = JsonConvert.DeserializeObject<{csharp_type}>(json);
        Assert.NotNull(obj);
        var output = JsonConvert.SerializeObject(obj);
        Assert.True(
            JToken.DeepEquals(JToken.Parse(json), JToken.Parse(output)),
            $"JSON mismatch:\nExpected: {{json}}\nActual: {{output}}"
        );
        File.WriteAllText(Path.Combine(BaseDir, "{name}.roundtrip.json"), output);
    }}"#
        ),
    }
}

/// Generates the full xUnit test class source.
fn generate_test_class(test_cases: &[TestCase], base_dir: &str, serializer: Serializer) -> String {
    let using_block = match serializer {
        Serializer::SystemTextJson => {
            "\
using System.Text.Json;
using Generated;
using Xunit;"
        }
        Serializer::Newtonsoft => {
            "\
using Newtonsoft.Json;
using Newtonsoft.Json.Linq;
using Generated;
using Xunit;"
        }
    };

    let facts: Vec<String> = test_cases
        .iter()
        .filter(|tc| !tc.csharp_definition.is_empty())
        .map(|tc| generate_xunit_fact(tc.name, tc.csharp_type, serializer))
        .collect();

    // Escape backslashes for C# string literal
    let escaped_dir = base_dir.replace('\\', "\\\\");

    format!(
        "using System.IO;\n{using_block}\n\npublic class RoundtripTests\n{{\n    private const string BaseDir = \"{escaped_dir}\";\n\n{facts}\n}}\n",
        facts = facts.join("\n\n")
    )
}

/// Generates the .csproj content for the xUnit test project.
fn generate_csproj(target_framework: &str, lang_version: &str, serializer: Serializer) -> String {
    let newtonsoft_pkg = if serializer == Serializer::Newtonsoft {
        "\n    <PackageReference Include=\"Newtonsoft.Json\" Version=\"13.*\" />"
    } else {
        ""
    };

    format!(
        r#"<Project Sdk="Microsoft.NET.Sdk">
  <PropertyGroup>
    <TargetFramework>{target_framework}</TargetFramework>
    <LangVersion>{lang_version}</LangVersion>
    <Nullable>enable</Nullable>
    <IsPackable>false</IsPackable>
    <IsTestProject>true</IsTestProject>
  </PropertyGroup>
  <ItemGroup>
    <PackageReference Include="xunit" Version="2.*" />
    <PackageReference Include="xunit.runner.visualstudio" Version="2.*" />
    <PackageReference Include="Microsoft.NET.Test.Sdk" Version="17.*" />{newtonsoft_pkg}
  </ItemGroup>
</Project>"#
    )
}

// ---------------------------------------------------------------------------
// Round-trip orchestrator
// ---------------------------------------------------------------------------

/// Runs the full bidirectional round-trip test for a given config.
///
/// # Panics
///
/// Panics if dotnet SDK is not available, if `dotnet test` fails,
/// or if Rust-side deserialization of the C#-produced JSON fails.
fn run_roundtrip(
    version: CSharpVersion,
    serializer: Serializer,
    lang_version: &str,
    target_framework: &str,
) {
    // 1. Check dotnet availability
    let dotnet_check = std::process::Command::new("dotnet")
        .arg("--version")
        .output();
    match dotnet_check {
        Ok(output) if output.status.success() => {}
        _ => panic!("dotnet SDK not found"),
    }

    let cfg = Config::default()
        .with_serializer(serializer)
        .with_target(version);

    let dir = std::env::temp_dir().join(format!(
        "csharp_rs_roundtrip_{version:?}_{lang_version}_{serializer:?}_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");

    // 2. Build test cases and write files
    let test_cases = build_test_cases(&cfg);

    for tc in &test_cases {
        // Write JSON input
        std::fs::write(dir.join(format!("{}.json", tc.name)), &tc.json)
            .unwrap_or_else(|e| panic!("write {}.json: {e}", tc.name));

        // Write C# type definition
        if !tc.csharp_definition.is_empty() {
            std::fs::write(dir.join(format!("{}.cs", tc.name)), &tc.csharp_definition)
                .unwrap_or_else(|e| panic!("write {}.cs: {e}", tc.name));
        }
    }

    // 3. Generate xUnit test class
    let base_dir = dir.to_string_lossy();
    let test_class = generate_test_class(&test_cases, &base_dir, serializer);
    std::fs::write(dir.join("RoundtripTests.cs"), &test_class).expect("write test class");

    // 4. Generate .csproj
    let csproj = generate_csproj(target_framework, lang_version, serializer);
    std::fs::write(dir.join("RoundtripTest.csproj"), &csproj).expect("write csproj");

    // 5. Run dotnet test
    let output = std::process::Command::new("dotnet")
        .args(["test", "--nologo", "--verbosity", "normal"])
        .env("DOTNET_SKIP_FIRST_TIME_EXPERIENCE", "1")
        .env("DOTNET_NOLOGO", "1")
        .env("DOTNET_CLI_TELEMETRY_OPTOUT", "1")
        .current_dir(&dir)
        .output()
        .expect("run dotnet test");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "dotnet test failed for {lang_version}/{serializer:?}:\nSTDOUT:\n{stdout}\nSTDERR:\n{stderr}"
    );

    // 6. Rust-side verification: deserialize C#-produced JSON back
    let roundtrip_json = std::fs::read_to_string(dir.join("BasicStruct.roundtrip.json"))
        .expect("read BasicStruct roundtrip");
    let roundtrip: BasicStruct = serde_json::from_str(&roundtrip_json).unwrap_or_else(|e| {
        panic!("deserialize BasicStruct roundtrip: {e}\nJSON: {roundtrip_json}")
    });
    assert_eq!(
        roundtrip,
        BasicStruct {
            name: "Alice".into(),
            level: 42,
            score: 99.5,
            active: true,
        },
        "BasicStruct round-trip mismatch"
    );

    let roundtrip_json = std::fs::read_to_string(dir.join("WithRenameAll.roundtrip.json"))
        .expect("read WithRenameAll roundtrip");
    let roundtrip: WithRenameAll = serde_json::from_str(&roundtrip_json).unwrap_or_else(|e| {
        panic!("deserialize WithRenameAll roundtrip: {e}\nJSON: {roundtrip_json}")
    });
    assert_eq!(
        roundtrip,
        WithRenameAll {
            player_name: "Charlie".into(),
            high_score: 99999,
        },
        "WithRenameAll round-trip mismatch"
    );

    let roundtrip_json = std::fs::read_to_string(dir.join("GenericContainer.roundtrip.json"))
        .expect("read GenericContainer roundtrip");
    let roundtrip: GenericContainer<String> =
        serde_json::from_str(&roundtrip_json).unwrap_or_else(|e| {
            panic!("deserialize GenericContainer roundtrip: {e}\nJSON: {roundtrip_json}")
        });
    assert_eq!(
        roundtrip,
        GenericContainer {
            item: "hello".to_string(),
            count: 7,
        },
        "GenericContainer round-trip mismatch"
    );

    let roundtrip_json = std::fs::read_to_string(dir.join("Message.roundtrip.json"))
        .expect("read Message roundtrip");
    let roundtrip: Message = serde_json::from_str(&roundtrip_json)
        .unwrap_or_else(|e| panic!("deserialize Message roundtrip: {e}\nJSON: {roundtrip_json}"));
    assert_eq!(
        roundtrip,
        Message::Request {
            id: "abc-123".into()
        },
        "Message round-trip mismatch"
    );

    // 7. Cleanup on success
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// Test matrix
// ---------------------------------------------------------------------------

#[test]
#[ignore = "requires dotnet SDK"]
fn roundtrip_unity_stj() {
    run_roundtrip(
        CSharpVersion::Unity,
        Serializer::SystemTextJson,
        "9.0",
        "net8.0",
    );
}

#[test]
#[ignore = "requires dotnet SDK"]
fn roundtrip_unity_newtonsoft() {
    run_roundtrip(
        CSharpVersion::Unity,
        Serializer::Newtonsoft,
        "9.0",
        "net8.0",
    );
}

#[test]
#[ignore = "requires dotnet SDK"]
fn roundtrip_csharp10_stj() {
    run_roundtrip(
        CSharpVersion::CSharp10,
        Serializer::SystemTextJson,
        "10.0",
        "net8.0",
    );
}

#[test]
#[ignore = "requires dotnet SDK"]
fn roundtrip_csharp10_newtonsoft() {
    run_roundtrip(
        CSharpVersion::CSharp10,
        Serializer::Newtonsoft,
        "10.0",
        "net8.0",
    );
}

#[test]
#[ignore = "requires dotnet SDK"]
fn roundtrip_csharp11_stj() {
    run_roundtrip(
        CSharpVersion::CSharp11,
        Serializer::SystemTextJson,
        "11.0",
        "net8.0",
    );
}
