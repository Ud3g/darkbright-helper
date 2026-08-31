fn main() {
    emit_git_describe();

    if std::env::var("CARGO_CFG_TARGET_OS").unwrap() == "windows" {
        let mut res = winres::WindowsResource::new();
        res.set_icon("res/icon.ico");
        // Bind the executable to version 6 of the common controls, which is
        // what turns on visual styles for system-drawn controls. Without it
        // Windows falls back to the pre-XP control renderer: buttons in the
        // app's message boxes are drawn in the grey 3D style, and a themed
        // control cannot follow the light/dark setting at all.
        //
        // Scope is deliberately just this dependency. Anything else a manifest
        // can declare — DPI awareness, long path support, an execution level —
        // changes how the app behaves rather than how a control is painted,
        // and belongs to its own change with its own testing.
        res.set_manifest(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <dependency>
    <dependentAssembly>
      <assemblyIdentity type="win32" name="Microsoft.Windows.Common-Controls" version="6.0.0.0" processorArchitecture="*" publicKeyToken="6595b64144ccf1df" language="*"/>
    </dependentAssembly>
  </dependency>
</assembly>"#,
        );
        res.compile().unwrap();
    }
}

/// Hands `git describe` output to the compiler as `DBH_GIT_DESCRIBE`, which
/// `core::version` turns into the string the tray menu and settings window
/// show.
///
/// The package version alone cannot distinguish a release from a build made
/// further along the same cycle — it does not change between releases. The
/// describe output can: on a released tag with a clean tree it is exactly the
/// tag, and anything else means the build has moved on.
///
/// Every failure here is silent and non-fatal: no git installed, no
/// repository (a source archive), no commits. The variable is then simply
/// absent and the app falls back to the package version, which is the right
/// answer for a build made from a release archive.
///
/// Deliberately emits no `rerun-if-changed`: doing so would replace Cargo's
/// default of re-running this script whenever any file in the package
/// changes, and that default is what keeps the recorded commit in step with
/// the code being compiled.
fn emit_git_describe() {
    let Ok(output) = std::process::Command::new("git")
        .args(["describe", "--tags", "--always", "--dirty"])
        .output()
    else {
        return;
    };
    if !output.status.success() {
        return;
    }
    let Ok(text) = String::from_utf8(output.stdout) else {
        return;
    };
    let describe = text.trim();
    // A stray newline would terminate the instruction early and turn the
    // remainder into a line Cargo cannot parse, so only single-line output is
    // passed on.
    if describe.is_empty() || describe.contains(['\n', '\r']) {
        return;
    }
    println!("cargo::rustc-env=DBH_GIT_DESCRIBE={describe}");
}
