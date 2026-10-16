import urllib.request,json,time,pathlib,datetime,hashlib
r=pathlib.Path('/home/user/.cache/redteam103/prs');r.mkdir(parents=True,exist_ok=True)
def get(path):
 u='https://api.github.com/repos/ms178/lccc/'+path+('&' if '?' in path else '?')+'refresh='+str(time.time_ns())
 req=urllib.request.Request(u,headers={'User-Agent':'lccc-empirical-audit','Accept':'application/vnd.github+json'})
 with urllib.request.urlopen(req,timeout=60) as f: raw=f.read()
 return json.loads(raw),u
main,_=get('commits/main'); opened=[]
for page in range(1,10):
 batch,_=get(f'pulls?state=open&per_page=100&page={page}');opened+=batch
 if len(batch)<100:break
else: raise RuntimeError('open pagination incomplete')
closed=[];urls=[]
for page in range(1,10):
 batch,u=get(f'pulls?state=closed&sort=updated&direction=desc&per_page=100&page={page}');closed+=batch;urls.append(u)
 selected=sorted(closed,key=lambda p:p['closed_at'],reverse=True)[:20]
 if len(selected)==20 and (len(batch)<100 or batch[-1]['updated_at']<=selected[-1]['closed_at']):break
else:raise RuntimeError('closed ordering unproven')
proof={'urls':urls,'fetched':len(closed),'last_updated_boundary':batch[-1]['updated_at'],'selected_closed_cutoff':selected[-1]['closed_at'],'ordering_proven':True,'selected':[p['number'] for p in selected]}
(r/'closed-order-proof.json').write_text(json.dumps(proof,indent=2))
(r/'inventory.json').write_text(json.dumps({'retrieved_utc':datetime.datetime.now(datetime.timezone.utc).isoformat(),'main':main['sha'],'open':opened,'closed':selected},indent=2))
print('MAIN',main['sha'],'OPEN',[p['number'] for p in opened],'CLOSED',proof['selected'],flush=True)
old=json.loads(pathlib.Path('/home/user/.cache/redteam94/prs/inventory.json').read_text());oldrows={p['number']:p for p in old['open']+old['closed']};status=[]
for p in opened+selected:
 n=p['number'];prior=oldrows.get(n);same=bool(prior and prior['head']['sha']==p['head']['sha'] and prior['updated_at']==p['updated_at']);row={'number':n,'head':p['head']['sha'],'state':p['state'],'unchanged_since_S97':same,'prior_archive':'artifacts/REDTEAM97-PR-REVIEW.tar.xz'};status.append(row)
 if not same:
  d=r/str(n);d.mkdir(exist_ok=True)
  for endpoint,leaf in [(f'pulls/{n}','metadata'),(f'pulls/{n}/reviews?per_page=100','reviews'),(f'pulls/{n}/comments?per_page=100','inline-comments'),(f'issues/{n}/comments?per_page=100','discussion')]:
   data,_=get(endpoint);(d/(leaf+'.json')).write_text(json.dumps(data,indent=2))
  req=urllib.request.Request(p['patch_url'],headers={'User-Agent':'lccc-empirical-audit'})
  with urllib.request.urlopen(req,timeout=120) as f:raw=f.read()
  (d/'pull.patch').write_bytes(raw);row['patch_sha256']=hashlib.sha256(raw).hexdigest();row['patch_bytes']=len(raw)
 print(n,'reuse captured revision' if same else 'refreshed',flush=True)
(r/'revision-check.json').write_text(json.dumps(status,indent=2))
