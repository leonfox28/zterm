CREATE PERFETTO TABLE ime_motion AS
SELECT id, ts, dur FROM slice WHERE name = 'InsetsAnimation: ime' AND dur > 0;

SELECT a.id, round(a.dur / 1e6, 2) AS duration_ms,
 count(*) AS frames,
 sum(f.jank_type GLOB '*App Deadline Missed*') AS app_deadline_misses,
 sum(f.jank_type GLOB '*Dropped Frame*') AS dropped,
 sum(f.jank_type GLOB '*Buffer Stuffing*') AS buffered,
 round(avg(f.dur) / 1e6, 3) AS frame_mean_ms
FROM ime_motion a JOIN actual_frame_timeline_slice f
 ON f.ts >= a.ts AND f.ts < a.ts + a.dur
WHERE f.layer_name GLOB '*io.github.leonfox28.zterm.dev*'
GROUP BY a.id;

SELECT s.name, count(*) AS n,
 round(avg(s.dur) / 1e6, 3) AS mean_ms,
 round(percentile(s.dur, 50) / 1e6, 3) AS p50_ms,
 round(percentile(s.dur, 95) / 1e6, 3) AS p95_ms
FROM slice s JOIN thread_track tt ON s.track_id = tt.id
JOIN thread t USING(utid)
WHERE t.upid IN (SELECT DISTINCT upid FROM actual_frame_timeline_slice
 WHERE layer_name GLOB '*io.github.leonfox28.zterm.dev*')
AND s.dur > 0 AND s.name IN ('Drawing  0.00  0.00 1200.00 2608.00',
 'renderFrame', 'flush commands', 'Record View#draw()',
 'AndroidOwner:measureAndLayout', 'AtlasTextOp', 'FillRectOp')
AND EXISTS (SELECT 1 FROM ime_motion a WHERE s.ts >= a.ts AND s.ts < a.ts+a.dur)
GROUP BY s.name ORDER BY sum(s.dur) DESC;
