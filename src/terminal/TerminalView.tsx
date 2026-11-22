import { useEffect, useRef } from 'react';
import { Terminal } from '@xterm/xterm';
import { FitAddon } from '@xterm/addon-fit';
import { WebglAddon } from '@xterm/addon-webgl';
import '@xterm/xterm/css/xterm.css';
import { b64ToBytes, bytesToB64 } from '../lib/b64';
import { request } from '../lib/ipc';
import { subscribeOutput } from '../lib/outputBus';
import { OutputSplicer } from './splice';
import { useStore } from '../store/store';

const cssVar = (name: string): string => getComputedStyle(document.documentElement).getPropertyValue(name).trim();

function termTheme() {
  const fg = cssVar('--term-fg');
  return {
    background: cssVar('--well'),
    foreground: fg,
    cursor: fg,
    cursorAccent: cssVar('--well'),
    selectionBackground: cssVar('--raised'),
    brightBlack: cssVar('--term-dim'),
  };
}

const encoder = new TextEncoder();

export function TerminalView({ sessionId }: { sessionId: string }) {
  const host = useRef<HTMLDivElement>(null);
  const termRef = useRef<Terminal | null>(null);
  const theme = useStore((s) => s.theme);

  useEffect(() => {
    const el = host.current;
    if (!el) return;
    const term = new Terminal({
      fontFamily: '"Geist Mono", ui-monospace, Menlo, monospace',
      fontSize: 12.5,
      cursorBlink: true,
      scrollback: 5000,
      allowProposedApi: true,
      theme: termTheme(),
    });
    termRef.current = term;
    const fit = new FitAddon();
    term.loadAddon(fit);
    term.open(el);
    try {
      const gl = new WebglAddon();
      gl.onContextLoss(() => gl.dispose());
      term.loadAddon(gl);
    } catch {
      // The DOM renderer is fine.
    }

    let disposed = false;
    const splicer = new OutputSplicer();
    const write = (bytes: Uint8Array) => term.write(bytes);
    // Subscribe first so nothing emitted during the attach round trip is lost.
    const off = subscribeOutput(sessionId, (bytes, offset) => splicer.push(bytes, offset, write));

    const sendSize = () => {
      try {
        fit.fit();
      } catch {
        return;
      }
      if (term.cols > 0 && term.rows > 0) {
        request('session_resize', { session_id: sessionId, cols: term.cols, rows: term.rows }).catch(() => {});
      }
    };

    sendSize();
    request<{ scrollback_b64: string; end: number }>('session_attach', { session_id: sessionId })
      .then((r) => {
        if (disposed) return;
        term.write(b64ToBytes(r.scrollback_b64));
        splicer.attach(r.end, write);
      })
      .catch((e: Error) => {
        if (!disposed) term.write(`\r\n${e.message}\r\n`);
      });

    const input = term.onData((data) => {
      request('session_input', { session_id: sessionId, data_b64: bytesToB64(encoder.encode(data)) }).catch(() => {});
    });
    const binary = term.onBinary((data) => {
      const bytes = Uint8Array.from(data, (c) => c.charCodeAt(0));
      request('session_input', { session_id: sessionId, data_b64: bytesToB64(bytes) }).catch(() => {});
    });

    const ro = new ResizeObserver(() => sendSize());
    ro.observe(el);

    return () => {
      disposed = true;
      ro.disconnect();
      input.dispose();
      binary.dispose();
      off();
      termRef.current = null;
      term.dispose();
    };
  }, [sessionId]);

  // Colors come from CSS variables, so re-read them when the theme flips.
  useEffect(() => {
    const term = termRef.current;
    if (term) term.options.theme = termTheme();
  }, [theme]);

  return (
    <div className="well terminal" role="log" aria-label="Terminal" style={{ padding: 8, overflow: 'hidden' }}>
      <div ref={host} style={{ width: '100%', height: '100%' }} />
    </div>
  );
}
