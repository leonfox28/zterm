CREATE PERFETTO TABLE ime_motion AS
SELECT id, ts, dur FROM slice WHERE name = 'InsetsAnimation: ime' AND dur > 0;

SELECT a.id, round((f.ts-a.ts)/1e6,3) AS start_ms,
 round(a.dur/1e6,3) AS animation_ms, round(f.dur/1e6,3) AS frame_ms,
 f.surface_frame_token, f.jank_type
FROM actual_frame_timeline_slice f JOIN ime_motion a
 ON f.ts>=a.ts AND f.ts<a.ts+a.dur
WHERE f.layer_name GLOB '*io.github.leonfox28.zterm.dev*'
 AND f.jank_type GLOB '*App Deadline Missed*'
ORDER BY f.ts;

CREATE PERFETTO TABLE app_presentations AS
SELECT DISTINCT a.id AS animation_id, f.display_frame_token,
 sf.ts+sf.dur AS present_ts
FROM ime_motion a JOIN actual_frame_timeline_slice f
 ON f.ts>=a.ts AND f.ts<a.ts+a.dur
JOIN actual_frame_timeline_slice sf
 ON sf.display_frame_token=f.display_frame_token
 AND sf.surface_frame_token IS NULL AND sf.dur>0
WHERE f.layer_name GLOB '*io.github.leonfox28.zterm.dev*'
 AND f.present_type!='Dropped Frame';

SELECT animation_id, count(*) AS presentations,
 round(avg(gap)/1e6,3) AS mean_gap_ms,
 round(percentile(gap,95)/1e6,3) AS p95_gap_ms,
 round(max(gap)/1e6,3) AS max_gap_ms,
 sum(gap>12500000) AS gaps_over_12_5_ms
FROM (SELECT *, present_ts-lag(present_ts) OVER
 (PARTITION BY animation_id ORDER BY present_ts) AS gap FROM app_presentations)
GROUP BY animation_id;

SELECT a.id, s.name, count(*) AS n, round(sum(s.dur)/1e6,3) AS sum_ms,
 round(max(s.dur)/1e6,3) AS max_ms
FROM actual_frame_timeline_slice f JOIN ime_motion a
 ON f.ts>=a.ts AND f.ts<a.ts+a.dur
JOIN slice s ON s.ts>=f.ts AND s.ts<f.ts+f.dur
JOIN thread_track tt ON s.track_id=tt.id JOIN thread t USING(utid)
WHERE f.layer_name GLOB '*io.github.leonfox28.zterm.dev*'
 AND f.jank_type GLOB '*App Deadline Missed*' AND t.upid=f.upid
 AND s.name IN ('Drawing  0.00  0.00 1200.00 2608.00',
 'renderFrame','flush commands','Record View#draw()',
 'AndroidOwner:measureAndLayout','AtlasTextOp','FillRectOp')
 AND s.dur>0
GROUP BY a.id,s.name ORDER BY a.id,sum_ms DESC;
