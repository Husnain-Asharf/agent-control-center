import { useEffect, useMemo, useRef, useState } from 'react';
import { clock } from '../lib/format';
import { nearest, niceMax, maxValue, sampleTimes, tableTimes, valueAt, xTicks, type RangeId, type Series } from '../lib/analytics';

/** Line colors, in series order. The UI stays grey: color is kept for session state. */
export const TONES = ['var(--ink)', 'var(--ink-3)', 'var(--ink-2)', 'color-mix(in srgb, var(--ink-3) 55%, transparent)'];

interface Props {
  title: string;
  series: Series[];
  since: number;
  now: number;
  range: RangeId;
  format: (v: number) => string;
  /** A fixed top for the axis, such as 100 for a share. */
  yMax?: number;
  /** A hairline at a value worth seeing, such as a limit. */
  mark?: { v: number; label: string };
  /** Rounds the axis top; the default suits plain numbers. */
  nice?: (v: number) => number;
  /** Fill under a single line. */
  area?: boolean;
}

const H = 168;
const TOP = 10;
const BOTTOM = 22;
const LEFT = 48;
const RIGHT = 64;
/** Wider right margin for line names, cut to fit. */
const RIGHT_NAMES = 92;
const cut = (s: string) => (s.length > 13 ? `${s.slice(0, 12)}…` : s);

const DAY = new Intl.DateTimeFormat(undefined, { weekday: 'short' });
const tickLabel = (t: number, range: RangeId) => (range === '7d' ? DAY.format(t) : clock(t));
const stamp = (t: number, range: RangeId) => (range === '7d' ? `${DAY.format(t)} ${clock(t)}` : clock(t));

function useWidth<T extends HTMLElement>(): [React.RefObject<T | null>, number] {
  const ref = useRef<T>(null);
  const [w, setW] = useState(0);
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    setW(el.clientWidth);
    if (typeof ResizeObserver === 'undefined') return;
    const ro = new ResizeObserver(() => setW(el.clientWidth));
    ro.observe(el);
    return () => ro.disconnect();
  }, []);
  return [ref, w];
}

/** A line chart with a crosshair, a legend for two or more lines, and a table view. */
export function TrendChart({ title, series, since, now, range, format, yMax, mark, nice = niceMax, area }: Props) {
  const [wrap, width] = useWidth<HTMLDivElement>();
  const [hover, setHover] = useState<number | null>(null);
  const [table, setTable] = useState(false);

  const top = yMax ?? nice(maxValue(series));
  const plotW = Math.max(0, width - LEFT - (series.length > 1 ? RIGHT_NAMES : RIGHT));
  const x = (t: number) => LEFT + ((t - since) / Math.max(1, now - since)) * plotW;
  const y = (v: number) => TOP + (1 - Math.min(v, top) / top) * H;
  const times = useMemo(() => sampleTimes(series), [series]);
  const yTicks = [0, top / 4, top / 2, (top * 3) / 4, top];

  const moveTo = (clientX: number, el: Element) => {
    const box = el.getBoundingClientRect();
    const t = since + ((clientX - box.left - LEFT) / Math.max(1, plotW)) * (now - since);
    setHover(nearest(times, t));
  };

  const onKey = (e: React.KeyboardEvent) => {
    if (!times.length || (e.key !== 'ArrowLeft' && e.key !== 'ArrowRight')) return;
    e.preventDefault();
    const i = hover == null ? times.length - 1 : times.indexOf(hover) + (e.key === 'ArrowLeft' ? -1 : 1);
    setHover(times[Math.max(0, Math.min(times.length - 1, i))]);
  };

  // End labels: the last value of each line, skipped where it would sit on another.
  const ends = series
    .map((s, i) => {
      const run = [...s.runs].reverse().find((r) => r.length);
      const p = run?.[run.length - 1];
      return p ? { i, p, label: series.length > 1 ? cut(s.label) : format(p.v) } : null;
    })
    .filter((e): e is NonNullable<typeof e> => e != null);
  const placed: number[] = [];
  const shown = ends.filter((e) => {
    const yy = y(e.p.v);
    if (placed.some((o) => Math.abs(o - yy) < 13)) return false;
    placed.push(yy);
    return true;
  });

  const tipX = hover == null ? 0 : x(hover);
  return (
    <figure className="an-chart">
      <figcaption className="an-chart-head">
        <h3>{title}</h3>
        {series.length > 1 && (
          <ul className="an-legend">
            {series.map((s, i) => (
              <li key={s.id}>
                <span className="an-key" style={{ background: TONES[i] }} />
                {s.label}
              </li>
            ))}
          </ul>
        )}
        <button type="button" className="btn btn--ghost an-table-btn" aria-pressed={table} onClick={() => setTable((v) => !v)}>
          {table ? 'Chart' : 'Table'}
        </button>
      </figcaption>
      {table ? (
        <ChartTable series={series} since={since} now={now} range={range} format={format} />
      ) : (
        <div className="an-plot" ref={wrap}>
          {width > 0 && (
            <svg
              width={width}
              height={TOP + H + BOTTOM}
              role="img"
              aria-label={`${title}. Use the left and right arrow keys to read values, or open the table.`}
              tabIndex={0}
              onKeyDown={onKey}
              onBlur={() => setHover(null)}
              onPointerMove={(e) => moveTo(e.clientX, e.currentTarget)}
              onPointerLeave={() => setHover(null)}
            >
              {yTicks.map((v) => (
                <g key={v}>
                  <line className="an-grid" x1={LEFT} x2={LEFT + plotW} y1={y(v)} y2={y(v)} />
                  <text className="an-tick" x={LEFT - 8} y={y(v) + 4} textAnchor="end">
                    {format(v)}
                  </text>
                </g>
              ))}
              {xTicks(since, now, range).map((t) => (
                <text key={t} className="an-tick" x={x(t)} y={TOP + H + 16} textAnchor="middle">
                  {tickLabel(t, range)}
                </text>
              ))}
              {mark && mark.v <= top && (
                <g>
                  <line className="an-mark" x1={LEFT} x2={LEFT + plotW} y1={y(mark.v)} y2={y(mark.v)} />
                  <text className="an-tick" x={LEFT + 4} y={y(mark.v) - 4}>
                    {mark.label}
                  </text>
                </g>
              )}
              {series.map((s, i) =>
                s.runs.map((run, k) => {
                  if (run.length === 1) {
                    return <circle key={`${s.id}-${k}`} cx={x(run[0].t)} cy={y(run[0].v)} r={2.5} fill={TONES[i]} />;
                  }
                  const d = run.map((p, j) => `${j ? 'L' : 'M'}${x(p.t).toFixed(1)},${y(p.v).toFixed(1)}`).join('');
                  return (
                    <g key={`${s.id}-${k}`}>
                      {area && series.length === 1 && (
                        <path
                          className="an-area"
                          d={`${d}L${x(run[run.length - 1].t).toFixed(1)},${y(0)}L${x(run[0].t).toFixed(1)},${y(0)}Z`}
                          style={{ fill: TONES[i] }}
                        />
                      )}
                      <path className="an-line" d={d} style={{ stroke: TONES[i] }} />
                    </g>
                  );
                }),
              )}
              {shown.map((e) => (
                <text key={e.i} className="an-end" x={x(e.p.t) + 8} y={y(e.p.v) + 4}>
                  {e.label}
                </text>
              ))}
              {hover != null && (
                <g>
                  <line className="an-cross" x1={tipX} x2={tipX} y1={TOP} y2={TOP + H} />
                  {series.map((s, i) => {
                    const v = valueAt(s, hover);
                    return v == null ? null : <circle key={s.id} className="an-dot" cx={tipX} cy={y(v)} r={4} style={{ fill: TONES[i] }} />;
                  })}
                </g>
              )}
            </svg>
          )}
          {hover != null && (
            <div
              className="an-tip"
              role="status"
              style={tipX > width / 2 ? { right: width - tipX + 12 } : { left: tipX + 12 }}
            >
              <div className="faint">{stamp(hover, range)}</div>
              {series.map((s, i) => {
                const v = valueAt(s, hover);
                return (
                  <div key={s.id} className="an-tip-row">
                    <span className="an-key" style={{ background: TONES[i] }} />
                    <strong>{v == null ? 'No sample' : format(v)}</strong>
                    {series.length > 1 && <span className="muted">{s.label}</span>}
                  </div>
                );
              })}
            </div>
          )}
        </div>
      )}
    </figure>
  );
}

function ChartTable({ series, since, now, range, format }: Pick<Props, 'series' | 'since' | 'now' | 'range' | 'format'>) {
  const times = tableTimes(since, now, 12);
  return (
    <div className="an-chart-table">
      <table className="utable">
        <thead>
          <tr>
            <th>Time</th>
            {series.map((s) => (
              <th key={s.id}>{s.label}</th>
            ))}
          </tr>
        </thead>
        <tbody>
          {times.map((t) => (
            <tr key={t}>
              <td>{stamp(t, range)}</td>
              {series.map((s) => {
                const v = valueAt(s, t);
                return <td key={s.id}>{v == null ? '' : format(v)}</td>;
              })}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}
