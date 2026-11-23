import { describe, expect, it } from 'vitest';
import raw from '../../src-tauri/capabilities/default.json?raw';
import baseRaw from '../../src-tauri/tauri.conf.json?raw';
import macosRaw from '../../src-tauri/tauri.macos.conf.json?raw';
import cargo from '../../src-tauri/Cargo.toml?raw';

const capability = JSON.parse(raw) as { windows: string[]; permissions: string[] };
type Conf = { app: { windows: Record<string, unknown>[]; macOSPrivateApi?: boolean } };
const base = JSON.parse(baseRaw) as Conf;
const macos = JSON.parse(macosRaw) as Conf;

describe('default capability', () => {
  it('covers the main window', () => {
    expect(capability.windows).toContain('main');
  });

  it('lets the title bar drag the window', () => {
    // data-tauri-drag-region calls start_dragging, which core:window:default does not allow.
    expect(capability.permissions).toContain('core:window:allow-start-dragging');
  });

  it('lets Settings zoom the webview for the text size', () => {
    // setZoom calls set_webview_zoom, which core:webview:default does not allow.
    expect(capability.permissions).toContain('core:webview:allow-set-webview-zoom');
  });

  it('lets the glass style turn window vibrancy on and off', () => {
    // setEffects, clearEffects and setTheme are not in core:window:default.
    expect(capability.permissions).toContain('core:window:allow-set-effects');
    expect(capability.permissions).toContain('core:window:allow-set-theme');
  });
});

describe('macOS window config', () => {
  it('is the main window, transparent for vibrancy', () => {
    // tauri.macos.conf.json replaces the windows list, so it must repeat the main window.
    const [main] = base.app.windows;
    const [mac] = macos.app.windows;
    expect(base.app.windows).toHaveLength(1);
    expect(macos.app.windows).toHaveLength(1);
    expect(mac).toEqual({ ...main, transparent: true });
  });

  it('enables the private API that transparent windows need', () => {
    expect(base.app.macOSPrivateApi).toBe(true);
    expect(cargo).toMatch(/^tauri = \{[^}]*features = \[[^\]]*"macos-private-api"/m);
  });

  it('keeps Windows and Linux windows opaque', () => {
    expect(base.app.windows[0].transparent).toBeUndefined();
  });
});
