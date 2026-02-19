{ pkgs ? import <nixpkgs> {} }:

pkgs.mkShell {
  packages = with pkgs; [
    cargo
    rustc
    rustfmt
    pkg-config
    gtk4
    webkitgtk_6_0
    glib-networking
    gst_all_1.gstreamer
    gst_all_1.gst-plugins-base
    gst_all_1.gst-plugins-good
  ];

  shellHook = ''
    extra_gio_modules="${pkgs.lib.makeSearchPath "lib/gio/modules" [ pkgs.glib-networking pkgs.dconf ]}"
    extra_gst_plugins="${pkgs.lib.makeSearchPath "lib/gstreamer-1.0" [
      pkgs.gst_all_1.gst-plugins-base
      pkgs.gst_all_1.gst-plugins-good
    ]}"
    if [ -n "$GIO_EXTRA_MODULES" ]; then
      export GIO_EXTRA_MODULES="$extra_gio_modules:$GIO_EXTRA_MODULES"
    else
      export GIO_EXTRA_MODULES="$extra_gio_modules"
    fi

    if [ -n "$GST_PLUGIN_SYSTEM_PATH_1_0" ]; then
      export GST_PLUGIN_SYSTEM_PATH_1_0="$extra_gst_plugins:$GST_PLUGIN_SYSTEM_PATH_1_0"
    else
      export GST_PLUGIN_SYSTEM_PATH_1_0="$extra_gst_plugins"
    fi
  '';
}
