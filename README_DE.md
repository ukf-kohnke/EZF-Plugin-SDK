# EZF-Plugin-SDK

[English](README.md) · [Deutsch](README_DE.md)

**EZF-Plugin-SDK** ist die gemeinsame Rust-Vertragsschicht für dynamisch geladene Plugins des EZF-Backends. Das Repository bündelt die Datenbank-Datenverträge, die Low-Level-Plugin-ABI und das Rust-seitige Plugin-SDK. Die drei Bestandteile bleiben dabei eigenständige Crates.

Das Repository ist für die Verwendung durch das EZF-Backend und dessen Plugins vorgesehen – einschließlich Plugins, die unabhängig vom Checkout des Backends gebaut werden.

## Crates

| Crate | Aufgabe |
| --- | --- |
| [`DatabaseContracts`](DatabaseContracts/) | Datenbankneutrale Typen für Anfragen, Antworten, Parameter, Ergebnisse, Werte und Fehler. Das Crate definiert die übergebenen Daten für Datenbankoperationen; es implementiert weder Datenbanktreiber noch Verbindungen. |
| [`PluginAbi`](PluginAbi/) | C-kompatible ABI-V2-Grundtypen für die Grenze zwischen dynamischen Bibliotheken, darunter Funktionstabellen, Byte-Slices und -Puffer, Host-Capabilities, Statuscodes und kooperative Cancellation. |
| [`PluginContracts`](PluginContracts/) | Rust-seitiges Plugin-SDK und logische Verträge: das `IPlugin`-Trait, Plugin-Manifeste und Endpoint-Beschreibungen, Katalogvalidierung, Lifecycle-Kontext, Endpoint-Request-/Response-Typen, Logging und Zugriff auf die vom Host bereitgestellte Datenbankausführung. |

Die Crates besitzen getrennte Verantwortlichkeiten und werden nicht zu einem einzigen Crate zusammengeführt. `PluginContracts` hängt von `DatabaseContracts` und `PluginAbi` ab und exportiert beide öffentlich erneut. Plugin-Implementierungen können ihre Typen dadurch aus derselben aufgelösten Contract-Abhängigkeit beziehen.

## Architektur

```text
EZF-Backend / PluginHost
          │
          │ Host-Capabilities und Lifecycle-Aufrufe
          ▼
   Plugin-ABI-V2-Grenze
   (C-kompatible Funktionstabelle + Byte-Puffer)
          │
          ▼
   PluginContracts ───────► DatabaseContracts
          │
          └───────────────► PluginAbi
```

An der Grenze zwischen dynamischen Bibliotheken werden bewusst keine Rust-spezifischen Datentypen wie `String`, `Vec`, Trait-Objekte oder Rust-Enums direkt über die ABI übergeben. Die Aufrufe verwenden C-kompatible Strukturen und explizit verwaltete Byte-Puffer; Contract-Payloads werden serialisiert. Puffer müssen über den zugehörigen ABI-Callback auf der Seite freigegeben werden, die sie angelegt hat.

### Zentrale Architekturmerkmale

- **Trennung von logischem und binärem Vertrag.** `PluginContracts` beschreibt die Rust-seitige Plugin-API; `PluginAbi` definiert die darunterliegende Schnittstelle zwischen Host und Plugin-Bibliothek.
- **Kooperative Cancellation.** ABI V2 übergibt bei potenziell lang laufenden Plugin-Operationen ein Cancellation-Signal pro Aufruf. Die Abbruchbehandlung ist kooperativ: Plugin-Code muss das Signal an geeigneten, sicheren Unterbrechungspunkten prüfen. Der Host beendet Plugin-Code nicht zwangsweise.
- **Datenbank-Capability statt Datenbankimplementierung.** `DatabaseContracts` definiert Anfragen und Ergebnisse. Die tatsächliche Ausführung stellt der Host bereit, sofern die Datenbank-Capability verfügbar ist.
- **Explizite Endpoint-Metadaten.** Plugin-Manifeste beschreiben Endpoint-Namen, Routen, HTTP-Methoden, Request-/Response-Medientypen und optionale Query- und Body-Metadaten. `PluginCatalog` validiert Manifeste und erkennt widersprüchliche Routen und Bezeichner.
- **Lifecycle-Verträge.** `IPlugin` beschreibt Plugin-Operationen und Lifecycle-Hooks einschließlich Laden, Entladen, Zerstören und Session-Close-Callbacks.

## Aktuelle Contract-Versionen

Das Repository enthält mehrere unabhängige Versionskennungen. Sie haben unterschiedliche Bedeutungen und dürfen nicht gleichgesetzt werden.

| Kennung | Aktueller Wert | Bedeutung |
| --- | ---: | --- |
| Workspace-/Paketversion | `1.0.0` | Cargo-Paketversion des eigenständigen Plugin-SDK-Workspaces. |
| `PLUGIN_CONTRACT_VERSION` | `8` | Version des logischen Plugin-Manifest-/API-Vertrags. |
| `PLUGIN_ABI_VERSION_V2` | `2` | Version der binären Plugin-ABI. |

Maßgeblich sind die Werte im Quellcode. Bei Änderungen am ABI-Layout, an Funktionssignaturen, an der Pufferverwaltung oder am Cancellation-Verhalten muss die Kompatibilität mit dem Host und allen betroffenen Plugins geprüft werden. Änderungen an serialisierten Plugin-Verträgen müssen ebenfalls gegen die Versionen von Host und Plugins getestet werden, die Daten miteinander austauschen.

## Bauen und testen

Erforderlich ist eine Rust-Toolchain mit Unterstützung für **Edition 2024**.

Im Stammverzeichnis des Repositories:

```bash
# Alle Crates des Workspaces kompilieren
cargo build --workspace

# Workspace-Tests ausführen
cargo test --workspace

# Formatierung prüfen
cargo fmt --all -- --check
```

Die aufgelösten Workspace-Abhängigkeiten lassen sich so anzeigen:

```bash
cargo tree --workspace
```

## EZF-Plugin-SDK einbinden

### Als Git-Abhängigkeit

Für ein Plugin-Repository, das unabhängig vom Checkout des EZF-Backends gebaut wird, kann `PluginContracts` aus diesem Repository eingebunden werden. Empfehlenswert ist ein geprüfter, unveränderlicher Commit (oder künftig ein Release-Tag, sobald das Projekt Tags verwendet) statt eines beweglichen Branches:

```toml
[dependencies]
PluginContracts = { git = "https://github.com/ukf-kohnke/EZF-Plugin-SDK.git", rev = "<vollständiger-commit-sha>" }
```

`PluginContracts` löst seine Geschwister-Crates aus demselben Repository-Checkout auf. Da `DatabaseContracts` und `PluginAbi` öffentlich re-exportiert werden, können Plugin-Implementierungen diese Typen über dieselbe Dependency importieren, beispielsweise:

```rust
use PluginContracts::DatabaseContracts::{
    DatabaseError,
    DatabaseRequest,
    DatabaseResponse,
};
use PluginContracts::PluginAbi;
use PluginContracts::ApiContract::{PluginManifest, PluginKind};
```

Vermeide eine zweite, unabhängig aufgelöste Kopie von `DatabaseContracts` oder `PluginAbi`, sofern es dafür keinen bewussten Grund gibt. Eine einheitliche Contract-Quelle reduziert das Risiko doppelter Crate-Identitäten und auseinanderlaufender Versionen.

### Als Submodule in EZF-Backend

Das Backend bindet dieses Repository als Git-Submodul unter `Plugin-SDK/` ein und verwendet für die integrierte Entwicklung lokale Pfadabhängigkeiten. Das Hauptrepository sollte das Submodul auf einen Commit pinnen, der mit den verwendeten Revisionen von `EZF-VariantFilter` und anderen Plugins kompatibel ist. Änderungen an gemeinsamen Typen sollten zusammen mit den betroffenen Plugins getestet werden.

## Hinweise zur Kompatibilität

- Den verwendeten Contract-Commit für jeden Plugin-Build pinnen und den zugehörigen Submodule-Commit im Backend-Repository festhalten.
- `EZF-Plugin-SDK` zunächst eigenständig testen und danach das integrierte Backend sowie die betroffenen Plugins bauen und testen.
- Änderungen an ABI und serialisierten Vertragstypen als kompatibilitätsrelevant behandeln. Identische Cargo-Paketversionen allein garantieren keine Kompatibilität.
- Cancellation ist kooperativ und kein Mechanismus, um beliebigen Plugin-Code zwangsweise zu beenden.
- Dieses Repository enthält Verträge und SDK-Code, nicht die Plugin-Host-Laufzeit, Datenbanktreiber oder anwendungsspezifische Plugin-Implementierungen.

## Lizenz

Die Cargo-Manifeste deklarieren eine duale Lizenzierung: wahlweise unter der [MIT License](https://opensource.org/license/mit) oder der [Apache License, Version 2.0](https://www.apache.org/licenses/LICENSE-2.0). Vor der Veröffentlichung sollten die zugehörigen Lizenztexte als Dateien im Repository enthalten sein.
