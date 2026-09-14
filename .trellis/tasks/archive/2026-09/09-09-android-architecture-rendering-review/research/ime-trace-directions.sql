CREATE PERFETTO TABLE ime_motion AS
SELECT a.id,a.ts,a.dur, CASE
 WHEN EXISTS(SELECT 1 FROM slice s WHERE s.name='IMS.showSoftInput'
  AND s.ts>=a.ts-100000000 AND s.ts<a.ts) THEN 'opening'
 WHEN EXISTS(SELECT 1 FROM slice s WHERE s.name='IMS.hideSoftInput'
  AND s.ts>=a.ts+a.dur AND s.ts<a.ts+a.dur+50000000) THEN 'closing'
 ELSE 'unknown' END AS direction
FROM slice a WHERE a.name='InsetsAnimation: ime' AND a.dur>0;

SELECT direction,count(*) AS animations FROM ime_motion GROUP BY direction;

SELECT a.direction,s.name,count(*) AS n,round(avg(s.dur)/1e6,3) AS mean_ms,
 round(percentile(s.dur,95)/1e6,3) AS p95_ms
FROM ime_motion a JOIN slice s ON s.ts>=a.ts AND s.ts<a.ts+a.dur
JOIN thread_track tt ON s.track_id=tt.id JOIN thread t USING(utid)
WHERE t.upid IN(SELECT DISTINCT upid FROM actual_frame_timeline_slice
 WHERE layer_name GLOB '*io.github.leonfox28.zterm.dev*')
AND s.dur>0 AND s.name IN ('Drawing  0.00  0.00 1200.00 2608.00',
 'renderFrame','Record View#draw()','AndroidOwner:measureAndLayout')
GROUP BY a.direction,s.name ORDER BY a.direction,s.name;
