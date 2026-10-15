#!/usr/bin/env python3
"""Collect the first 20 merged-PR observations after a policy change.

Read-only GitHub REST requests. A failed attempt followed by success is a rerun
recovery SIGNAL, not a diagnosed flake. Missing/in-progress evidence stays
explicitly incomplete. No timing gate or automatic policy changes.
"""
import argparse
from datetime import datetime, timezone
import json
import os
from pathlib import Path
from urllib.request import Request, urlopen


def github_get(route):
    headers={'Accept':'application/vnd.github+json','X-GitHub-Api-Version':'2022-11-28','User-Agent':'lccc-ci-followup'}
    if token:=os.environ.get('GITHUB_TOKEN'):headers['Authorization']='Bearer '+token
    with urlopen(Request('https://api.github.com'+route,headers=headers),timeout=30) as response:
        return json.load(response)


def pages(get,route,key=None):
    result=[]
    for page in range(1,101):
        data=get(route+('&' if '?' in route else '?')+f'per_page=100&page={page}')
        items=data[key] if key else data
        result.extend(items)
        if len(items)<100:return result
    raise ValueError('pagination limit reached; dataset is incomplete')


def seconds(start,end):
    if not start or not end:return None
    return (datetime.fromisoformat(end.replace('Z','+00:00'))-datetime.fromisoformat(start.replace('Z','+00:00'))).total_seconds()


def collect(repo,after,get=github_get):
    prefix='/repos/'+repo
    baseline=get(f'{prefix}/pulls/{after}')
    cutoff=baseline.get('merged_at')
    if not cutoff:raise ValueError('baseline PR is not merged')
    result={'schema_version':1,'repository':repo,'after_pr':after,'after_merged_at':cutoff,
            'captured_at':datetime.now(timezone.utc).isoformat(),'required_merged_prs':20,
            'complete':False,'status':'awaiting-20-merged-PRs','prs':[],'runs':[],
            'limits':'PR merge order is a cohort, not proof of identical workflow source. Rerun recovery is not a confirmed flake. Durations include runner effects.'}
    closed=[]
    for page in range(1,101):
        chunk=get(f'{prefix}/pulls?state=closed&sort=updated&direction=desc&per_page=100&page={page}')
        closed.extend(p for p in chunk if p.get('merged_at') and p['merged_at']>cutoff and p['base']['ref']=='main')
        if len(chunk)<100 or chunk[-1]['updated_at']<cutoff:break
    else:raise ValueError('PR pagination limit reached')
    result['eligible_merged_prs']=len(closed)
    cohort=sorted(closed,key=lambda p:(p['merged_at'],p['number']))[:20]
    result['prs']=[{'number':p['number'],'head_sha':p['head']['sha'],'merged_at':p['merged_at']} for p in cohort]
    if len(cohort)<20:return result
    complete=True
    for pr in result['prs']:
        runs=pages(get,f"{prefix}/actions/runs?head_sha={pr['head_sha']}&event=pull_request",'workflow_runs')
        runs=[r for r in runs if r['path']=='.github/workflows/ci.yml' and r['head_sha']==pr['head_sha']]
        if not runs:complete=False
        for run in runs:
            record={'pr':pr['number'],'run_id':run['id'],'head_sha':run['head_sha'],'workflow_path':run['path'],'url':run['html_url'],'attempts':[]}
            for attempt in range(1,run['run_attempt']+1):
                route=f"{prefix}/actions/runs/{run['id']}/attempts/{attempt}"
                detail=get(route)
                jobs=pages(get,route+'/jobs','jobs')
                record['attempts'].append({'attempt':attempt,'status':detail['status'],'conclusion':detail['conclusion'],
                    'started_at':detail.get('run_started_at'),'updated_at':detail['updated_at'],
                    'elapsed_to_update_s':seconds(detail.get('run_started_at'),detail['updated_at']),
                    'jobs':[{'id':j['id'],'name':j['name'],'conclusion':j['conclusion'],'status':j['status'],
                             'started_at':j.get('started_at'),'completed_at':j.get('completed_at'),
                             'duration_s':seconds(j.get('started_at'),j.get('completed_at')),'url':j['html_url']} for j in jobs]})
                if detail['status']!='completed' or any(j['status']!='completed' for j in jobs):complete=False
            conclusions=[a['conclusion'] for a in record['attempts']]
            record['rerun_recovery_signal']='failure' in conclusions[:-1] and conclusions[-1]=='success'
            result['runs'].append(record)
    result['complete']=complete
    result['status']='observations-collected-root-cause-review-required' if complete else 'incomplete-run-observations'
    return result


def main(argv=None):
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--repo',default='ms178/lccc')
    parser.add_argument('--after-pr',type=int,default=834)
    parser.add_argument('--out',type=Path,required=True)
    args=parser.parse_args(argv)
    try:result=collect(args.repo,args.after_pr)
    except (OSError,ValueError,KeyError,TypeError) as error:
        result={'complete':False,'status':'collection-error','error':str(error)}
    args.out.parent.mkdir(parents=True,exist_ok=True)
    args.out.write_text(json.dumps(result,indent=2)+'\n')
    print(result['status'])
    return 0 if result.get('complete') else 2


if __name__=='__main__':raise SystemExit(main())
