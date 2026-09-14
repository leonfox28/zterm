-- Use with trace_processor query -f on the explicitly instrumented emulator traces.
-- The slow-animation run controls candidate ordering; it is not a FPS comparison.
CREATE PERFETTO TABLE ime_motion AS
SELECT a.id, a.ts, a.dur FROM slice a WHERE a.name = 'InsetsAnimation: ime'
 AND a.dur > 0 AND EXISTS (SELECT 1 FROM slice p
 WHERE p.name GLOB 'Zterm.ime.*' AND a.ts BETWEEN p.ts AND p.ts + p.dur);

SELECT name, count(*) AS n, round(avg(dur)/1e6,3) AS mean_ms,
 round(percentile(dur,95)/1e6,3) AS p95_ms, round(max(dur)/1e6,3) AS max_ms
FROM slice WHERE name GLOB 'Zterm.*' AND name NOT GLOB 'Zterm.ime.*' AND dur > 0
GROUP BY name ORDER BY sum(dur) DESC;

SELECT a.id, round(a.ts/1e6,2) AS start_ms, round(a.dur/1e6,2) AS ime_ms,
 round(d.dur/1e6,3) AS draw_ms, round((d.ts-a.ts-a.dur)/1e6,3) AS end_delta_ms,
 s.name, round(s.dur/1e6,3) AS stage_ms,
 (SELECT count(*) FROM slice r WHERE r.name='Zterm.rowRecord'
  AND r.ts BETWEEN d.ts AND d.ts+d.dur) AS recorded_rows
FROM ime_motion a JOIN slice d ON d.name='Zterm.draw'
 AND d.ts BETWEEN a.ts+a.dur-5000000 AND a.ts+a.dur+100000000
JOIN slice s ON s.parent_id=d.id ORDER BY a.ts,d.ts,s.ts;

CREATE PERFETTO TABLE row_counts AS
SELECT c.ts, c.value, t.name FROM counter c JOIN counter_track t ON c.track_id=t.id
WHERE t.name IN ('Zterm.currentRows','Zterm.drawnRows');
CREATE PERFETTO TABLE ime_initial AS
SELECT a.*, coalesce(
 (SELECT value FROM row_counts c WHERE c.name='Zterm.drawnRows'
 AND c.ts <= a.ts ORDER BY c.ts DESC LIMIT 1),
 (SELECT value FROM row_counts c WHERE c.name='Zterm.drawnRows'
 AND c.ts BETWEEN a.ts AND a.ts+a.dur ORDER BY c.ts LIMIT 1)) AS old_rows
FROM ime_motion a;

SELECT a.id, a.old_rows, round(a.dur/1e6,2) AS ime_ms,
 round(((SELECT min(ts) FROM row_counts c WHERE c.name='Zterm.currentRows'
  AND c.ts BETWEEN a.ts AND a.ts+a.dur AND c.value!=a.old_rows)-a.ts)/1e6,3) AS candidate_after_start_ms,
 round(((SELECT min(ts) FROM row_counts c WHERE c.name='Zterm.drawnRows'
  AND c.ts BETWEEN a.ts AND a.ts+a.dur+100000000 AND c.value!=a.old_rows)-a.ts-a.dur)/1e6,3) AS handoff_after_end_ms
FROM ime_initial a;

SELECT s.id, s.name, round(s.dur/1e6,3) AS wall_ms,
 round(sum(max(0,min(r.ts+r.dur,s.ts+s.dur)-max(r.ts,s.ts)))/1e6,3) AS running_ms
FROM slice s JOIN thread_track t ON s.track_id=t.id
LEFT JOIN sched_slice r ON r.utid=t.utid AND r.ts<s.ts+s.dur AND r.ts+r.dur>s.ts
WHERE s.name IN ('Zterm.anchor','Zterm.rows','Zterm.source') AND s.dur>1000000
GROUP BY s.id ORDER BY s.dur DESC LIMIT 20;
