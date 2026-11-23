import { describe, expect, it } from 'vitest';
import raw from '../../src-tauri/capabilities/default.json?raw';

const capability = JSON.parse(raw) as { windows: string[]; permissions: string[] };

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
});
