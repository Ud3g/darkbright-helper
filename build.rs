fn main() {
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
