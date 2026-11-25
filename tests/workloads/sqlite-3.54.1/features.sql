-- Exercise SQLite extensions and less common optimizer/execution paths.
PRAGMA foreign_keys=ON;
CREATE VIRTUAL TABLE docs USING fts5(body, tokenize='unicode61');
INSERT INTO docs(body) VALUES('shader cache, быстрый поиск'),
  ('cloud state and shader compilation'),('save files and backup');
SELECT CASE WHEN count(*)=2 THEN 'PASS|fts5' ELSE 'FAIL|fts5' END
  FROM docs WHERE docs MATCH 'shader';
SELECT rowid, highlight(docs,0,'[',']') FROM docs WHERE docs MATCH 'shader' ORDER BY rowid;
CREATE VIRTUAL TABLE bbox USING rtree(id,minX,maxX,minY,maxY);
INSERT INTO bbox VALUES(1,0,3,0,3),(2,1,4,1,4),(3,9,12,9,12);
SELECT CASE WHEN count(*)=2 THEN 'PASS|rtree' ELSE 'FAIL|rtree' END
  FROM bbox WHERE minX<=2 AND maxX>=2 AND minY<=2 AND maxY>=2;
CREATE TABLE kv(k TEXT PRIMARY KEY, v TEXT NOT NULL) WITHOUT ROWID;
INSERT INTO kv VALUES('x','{"values":[1,2,3]}');
INSERT INTO kv VALUES('x','{"values":[4,5]}') ON CONFLICT(k) DO UPDATE SET v=excluded.v;
SELECT CASE WHEN json_extract(v,'$.values[0]')=4
  THEN 'PASS|json-upsert' ELSE 'FAIL|json-upsert' END FROM kv;
SELECT json_array_length(v,'$.values'), json_type(v,'$.values'), json_valid(v) FROM kv;
CREATE TABLE typed(i INTEGER NOT NULL, text TEXT) STRICT;
INSERT INTO typed VALUES(7,'abc'),(9,'déjà vu');
SELECT CASE WHEN sum(i)=16 AND count(*)=2 THEN 'PASS|strict' ELSE 'FAIL|strict' END FROM typed;
WITH RECURSIVE fib(n,a,b) AS (
  VALUES(0,0,1) UNION ALL SELECT n+1,b,a+b FROM fib WHERE n<25
)
SELECT CASE WHEN b=121393 THEN 'PASS|cte' ELSE 'FAIL|cte' END FROM fib WHERE n=25;
SELECT i, lead(i) OVER(ORDER BY i), sum(i) OVER(ORDER BY i ROWS BETWEEN 1 PRECEDING AND CURRENT ROW)
  FROM typed ORDER BY i;
SELECT CASE WHEN (SELECT count(*) FROM pragma_foreign_key_check)=0
  THEN 'PASS|foreign-keys' ELSE 'FAIL|foreign-keys' END;
PRAGMA integrity_check;
