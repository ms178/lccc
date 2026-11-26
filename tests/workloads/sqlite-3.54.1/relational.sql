-- Deterministic SQL differential: indexes, joins, aggregates, transactions,
-- triggers, UTF-8, ordering, floating point, window functions and integrity.
PRAGMA foreign_keys=ON;
PRAGMA journal_mode=MEMORY;
PRAGMA synchronous=OFF;
CREATE TABLE item(id INTEGER PRIMARY KEY, name TEXT NOT NULL, value REAL, state TEXT);
WITH RECURSIVE n(i) AS (VALUES(1) UNION ALL SELECT i+1 FROM n WHERE i<4096)
INSERT INTO item SELECT i, printf('élément_%04d',i), i/3.0,
  CASE i%4 WHEN 0 THEN 'new' WHEN 1 THEN 'ready' WHEN 2 THEN 'done' ELSE 'cached' END FROM n;
CREATE INDEX item_state_value ON item(state,value DESC);
CREATE TABLE event(item_id INTEGER, kind TEXT, payload BLOB,
  FOREIGN KEY(item_id) REFERENCES item(id));
INSERT INTO event SELECT id, CASE id%3 WHEN 0 THEN 'launch' WHEN 1 THEN 'shader' ELSE 'network' END,
  zeroblob(32+(id%5)) FROM item;
CREATE INDEX event_item_kind ON event(item_id,kind);
SELECT CASE WHEN count(*)=4096 AND sum(id)=8390656 THEN 'PASS|rows' ELSE 'FAIL|rows' END FROM item;
SELECT state, count(*), printf('%.3f',sum(value)) FROM item GROUP BY state ORDER BY state;
SELECT i.name,count(e.item_id) FROM item i LEFT JOIN event e ON e.item_id=i.id
  GROUP BY i.id HAVING count(e.item_id)>0 ORDER BY i.id LIMIT 16;
SELECT name FROM item WHERE state='ready' AND value BETWEEN 100.0 AND 500.0
  ORDER BY value DESC LIMIT 16;
SELECT item_id FROM event WHERE kind='shader'
  INTERSECT SELECT id FROM item WHERE state='done' ORDER BY item_id LIMIT 16;
SELECT state, id, row_number() OVER(PARTITION BY state ORDER BY value DESC)
  FROM item WHERE id BETWEEN 100 AND 120 ORDER BY state,id;
CREATE TABLE audit(n INTEGER NOT NULL);
CREATE TRIGGER item_audit AFTER UPDATE ON item BEGIN INSERT INTO audit VALUES(new.id); END;
BEGIN;
UPDATE item SET state='ready' WHERE id BETWEEN 1 AND 5;
SELECT CASE WHEN (SELECT count(*) FROM audit)=5 THEN 'PASS|triggers' ELSE 'FAIL|triggers' END;
ROLLBACK;
SELECT CASE WHEN (SELECT count(*) FROM audit)=0 AND (SELECT state FROM item WHERE id=4)='new'
  THEN 'PASS|rollback' ELSE 'FAIL|rollback' END;
SELECT CASE WHEN (SELECT count(*) FROM pragma_foreign_key_check)=0
  THEN 'PASS|foreign-keys' ELSE 'FAIL|foreign-keys' END;
PRAGMA integrity_check;
