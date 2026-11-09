#!/usr/bin/env python3
"""One-time cutover from task mirrors to Calendar tasks; never imported by the app.

Stop every old TimeSlay client before --apply. Snapshot/plan is the default.
Backups and a resumable journal are written before any destructive operation.
"""
import argparse, copy, datetime, hashlib, json, os, pathlib, re, secrets, shutil, sqlite3, time, tomllib
import urllib.error, urllib.parse, urllib.request

CAL = 'https://www.googleapis.com/calendar/v3'
TASK = 'https://tasks.googleapis.com/tasks/v1'

def compact(v): return json.dumps(v, separators=(',', ':'), ensure_ascii=False)
def enc(v): return urllib.parse.quote(v, safe='')
def footer(notes):
    parts = re.split(r'(?:^|\n)---\ntt-version: ', notes)
    if len(parts) == 1: return notes, None
    if len(parts) != 2: raise ValueError('Ambiguous task metadata')
    lines = ('tt-version: '+parts[1]).splitlines()
    fields = {k[3:]: json.loads(v) for k,v in (line.split(': ',1) for line in lines)}
    if fields.pop('version') != 1: raise ValueError('Unsupported task metadata')
    task_id = fields.pop('id'); deadline = fields.pop('deadline', None)
    prose = parts[0]
    if deadline: prose += '\nDEADLINE: '+deadline
    fields.pop('schedule',None)
    return prose.strip(), {'id':task_id,'metadata':fields}

def properties(data, projects, labels, completed):
    raw=compact(data)
    # Google limits each extended-property value to 1024 characters.
    chunks=[];chunk=''
    for char in raw:
        if len((chunk+char).encode())>900:chunks.append(chunk);chunk=''
        chunk+=char
    if chunk:chunks.append(chunk)
    result={'ttKind':'task','ttDataCount':str(len(chunks)), 'ttCompleted':str(completed).lower(),
            'ttProjects':compact(projects),'ttLabels':compact(labels)}
    result.update({'ttData'+str(i):v for i,v in enumerate(chunks)})
    return result

class Migration:
    def __init__(self,args):
        self.args=args; self.root=pathlib.Path(args.backup);self.root.mkdir(parents=True,exist_ok=True);self.root.chmod(0o700)
        self.config_path=pathlib.Path(args.config); self.config=tomllib.loads(self.config_path.read_text())
        self.db_path=pathlib.Path(args.database)
        sec=json.loads(pathlib.Path(args.secrets).read_text())['google_tokens']
        self.token=sec['access_token']
        if sec.get('expires_at',0)<time.time()+120:
            fields={'grant_type':'refresh_token','refresh_token':sec['refresh_token'],
                    'client_id':self.config['calendar']['client_id'],'client_secret':self.config['calendar']['client_secret']}
            req=urllib.request.Request('https://oauth2.googleapis.com/token',data=urllib.parse.urlencode(fields).encode())
            self.token=json.load(urllib.request.urlopen(req,timeout=45))['access_token']
    def save(self,name,value):
        p=self.root/name
        tmp=p.with_suffix('.tmp');tmp.write_text(json.dumps(value,indent=2,ensure_ascii=False));tmp.chmod(0o600);tmp.replace(p)
    def request(self,method,url,body=None):
        req=urllib.request.Request(url,method=method,headers={'Authorization':'Bearer '+self.token,'Content-Type':'application/json'},data=None if body is None else compact(body).encode())
        for attempt in range(5):
            try:
                with urllib.request.urlopen(req,timeout=45) as r:
                    raw=r.read();return json.loads(raw) if raw else {}
            except urllib.error.HTTPError as e:
                if method=='DELETE' and e.code in (404,410):return {}
                if e.code in (429,500,502,503,504) and attempt<4:time.sleep(2**attempt);continue
                raise RuntimeError(f'{method} {url.split("?")[0]}: HTTP {e.code}') from None
    def pages(self,url):
        items=[]
        while True:
            page=self.request('GET',url);items.extend(page.get('items',[]))
            token=page.get('nextPageToken')
            if not token:return items
            url=re.sub(r'&pageToken=[^&]*','',url)+'&pageToken='+enc(token)
    def snapshot(self):
        path=self.root/'snapshot.json'
        if path.exists():return json.loads(path.read_text())
        con=sqlite3.connect('file:'+str(self.db_path)+'?mode=ro',uri=True)
        out=sqlite3.connect(self.root/'before.sqlite3');con.backup(out);out.close();con.close();(self.root/'before.sqlite3').chmod(0o600)
        (self.root/'config-before.toml').write_bytes(self.config_path.read_bytes());(self.root/'config-before.toml').chmod(0o600)
        calendars=self.pages(CAL+'/users/me/calendarList?maxResults=250')
        lists=self.pages(TASK+'/users/@me/lists?maxResults=100')
        tasks={l['id']:self.pages(TASK+'/lists/'+enc(l['id'])+'/tasks?maxResults=100&showCompleted=true&showHidden=true') for l in lists}
        events={c['id']:self.pages(CAL+'/calendars/'+enc(c['id'])+'/events?maxResults=2500&singleEvents=false&privateExtendedProperty=ttKind%3Dtask') for c in calendars}
        snapshot={'calendars':calendars,'lists':lists,'tasks':tasks,'events':events};self.save('snapshot.json',snapshot);return snapshot
    def plan(self,snap):
        path=self.root/'plan.json'
        if path.exists():return json.loads(path.read_text())
        con=sqlite3.connect(self.root/'before.sqlite3')
        lookup={source:json.loads(meta)|{'_id':tid} for source,tid,meta in con.execute('SELECT a.source,t.id,t.metadata FROM task_aliases a JOIN tasks t ON t.id=a.task_id')};con.close()
        settings=self.config['calendar'];app=settings.get('app_calendar') or next(c['id'] for c in snap['calendars'] if c.get('summary')=='TimeSlay')
        old={l['id'] for l in snap['lists'] if l['title'].strip().lower() in ('time blocked','timme blocked')}
        if settings.get('time_block_list'):old.add(settings['time_block_list'].removeprefix('tasks:'))
        mirrors={};orphans=[]
        for cid,events in snap['events'].items():
            for event in events:
                p=event.get('extendedProperties',{}).get('private',{})
                if 'ttDataCount' in p:continue
                key=(p.get('ttTaskList'),p.get('ttTask'))
                mirrors.setdefault(key,[]).append((cid,event))
        actions=[];handled=set()
        for lid,tasks in snap['tasks'].items():
            for task in tasks:
                key=(lid,task['id']);copies=mirrors.get(key,[])
                notes,data=footer(task.get('notes',''))
                alias='tasks:'+lid+'/task:'+task['id']
                if data is None:
                    meta=copy.deepcopy(lookup.get(alias,{}));tid=meta.pop('_id',None) or secrets.randbelow(2**62-100)+100;meta.pop('schedule',None)
                    data={'id':tid,'metadata':meta}
                metadata=data['metadata'];metadata.pop('schedule',None)
                if not copies and not task.get('due'):
                    if lid in old: actions.append({'kind':'inbox','list':lid,'task':task,'data':data})
                    continue
                # Select the exact reserved mirror where available, otherwise preserve all independent intervals.
                for cid,event in copies or [(app,None)]:
                    eid=event['id'] if event else 'tt'+hashlib.sha256(alias.encode()).hexdigest()[:32]
                    start=event.get('start') if event else {'date':task['due'][:10]}
                    end=event.get('end') if event else {'date':(datetime.date.fromisoformat(task['due'][:10])+datetime.timedelta(days=1)).isoformat()}
                    body={'summary':task.get('title',''),'description':notes,'start':start,'end':end,
                          'extendedProperties':{'private':properties(data,metadata.get('projects',[]),metadata.get('labels',[]),task.get('status')=='completed')}}
                    if event:
                        for k in ('recurrence','reminders','transparency','visibility','location'):
                            if k in event:body[k]=event[k]
                        for name in ('ttTaskList','ttTask','ttBlock','ttOrder','ttTaskLink','ttDeadline'):
                            body['extendedProperties']['private'][name]=None
                    actions.append({'kind':'calendar','list':lid,'task':task,'calendar':cid,'id':eid,'body':body,'data':data})
                handled.add(key)
        for key,copies in mirrors.items():
            if key in handled:continue
            for cid,event in copies:
                notes,data=footer(event.get('description',''));p=event.get('extendedProperties',{}).get('private',{})
                if data is None:
                    meta=copy.deepcopy(lookup.get('tasks:'+str(key[0])+'/task:'+str(key[1]),{}));tid=meta.pop('_id',None) or secrets.randbelow(2**62-100)+100;meta.pop('schedule',None)
                    meta['block']=p.get('ttBlock') or meta.get('block');meta['order']=int(p.get('ttOrder',0));data={'id':tid,'metadata':meta}
                if p.get('ttDeadline') and not re.search(r'^DEADLINE:',notes,re.M):notes+='\nDEADLINE: '+p['ttDeadline']
                for prop,field in [('ttProjects','projects'),('ttLabels','labels')]:
                    if p.get(prop) and not data['metadata'].get(field):data['metadata'][field]=json.loads(p[prop])
                if event.get('summary','').startswith('Ignored · '):data['metadata']['ignored']=True
                data['metadata'].setdefault('minutes',0)
                if 'dateTime' in event.get('start',{}) and 'dateTime' in event.get('end',{}):
                    start=datetime.datetime.fromisoformat(event['start']['dateTime']);end=datetime.datetime.fromisoformat(event['end']['dateTime'])
                    data['metadata']['minutes']=max(0,int((end-start).total_seconds()/60))
                body={'summary':event.get('summary','').removeprefix('✓ ').removeprefix('Ignored · '),'description':notes,
                      'extendedProperties':{'private':properties(data,data['metadata'].get('projects',[]),data['metadata'].get('labels',[]),p.get('ttCompleted')=='true')}}
                for name in ('ttTaskList','ttTask','ttBlock','ttOrder','ttTaskLink','ttDeadline'):body['extendedProperties']['private'][name]=None
                actions.append({'kind':'orphan','calendar':cid,'id':event['id'],'body':body,'data':data})
        plan={'actions':actions,'remove_lists':sorted(old),'app_calendar':app};self.save('plan.json',plan);return plan
    def apply(self,plan):
        journal_path=self.root/'journal.json';journal=json.loads(journal_path.read_text()) if journal_path.exists() else {}
        for index,action in enumerate(plan['actions']):
            key=str(index)
            if key in journal:continue
            if action['kind'] in ('calendar','orphan','series-source'):
                url=CAL+'/calendars/'+enc(action['calendar'])+'/events/'+enc(action['id'])
                try:self.request('GET',url);self.request('PATCH',url,action['body'])
                except RuntimeError as e:
                    if not str(e).endswith('HTTP 404'):raise
                    self.request('POST',CAL+'/calendars/'+enc(action['calendar'])+'/events',action['body']|{'id':action['id']})
                remote=self.request('GET',url)
                if 'recurrence' in action['body']:assert remote.get('recurrence')==action['body']['recurrence']
                for field in ('summary','description'):
                    assert remote.get(field,'')==action['body'].get(field,''),field
                actual=remote['extendedProperties']['private'];expected=action['body']['extendedProperties']['private']
                for prop,value in expected.items():
                    if value is not None:assert actual[prop]==value,prop
                journal[key]={'calendar':action['calendar'],'id':action['id']}
            else:
                inbox=self.config['calendar'].get('inbox_list','').removeprefix('tasks:')
                if not inbox:
                    lists=self.pages(TASK+'/users/@me/lists?maxResults=100');inbox=next((l['id'] for l in lists if l['title']=='Inbox'),None)
                if not inbox:inbox=self.request('POST',TASK+'/users/@me/lists',{'title':'Inbox'})['id']
                prose,_=footer(action['task'].get('notes',''))
                data=action['data']; meta=data['metadata']
                notes=prose+'\n---\ntt-version: 1\ntt-id: '+str(data['id'])
                deadline=re.search(r'^DEADLINE: (.+)$',prose,re.M)
                notes=re.sub(r'^DEADLINE: .+\n?','',prose,flags=re.M).rstrip()+'\n---\ntt-version: 1\ntt-id: '+str(data['id'])
                notes+='\ntt-deadline: '+compact(deadline.group(1) if deadline else None)
                for field,value in meta.items():notes+='\ntt-'+field+': '+compact(value)
                self.request('PATCH',TASK+'/lists/'+enc(action['list'])+'/tasks/'+enc(action['task']['id']),{'notes':notes,'due':None})
                # Cross-list Tasks.move retains the original resource's notes and status.
                result=self.request('POST',TASK+'/lists/'+enc(action['list'])+'/tasks/'+enc(action['task']['id'])+'/move?destinationTasklist='+enc(inbox))
                journal[key]={'list':inbox,'id':result['id']}
            self.save('journal.json',journal)
        # Publish and verify every destination before removing any source.
        self.save('destinations-verified.json',journal)
        source_ids={}
        for lid in {a['list'] for a in plan['actions'] if a['kind']=='calendar' and a['list'] not in plan['remove_lists']}:
            try:source_ids[lid]={t['id'] for t in self.pages(TASK+'/lists/'+enc(lid)+'/tasks?maxResults=100&showCompleted=true&showHidden=true')}
            except RuntimeError as e:
                if not str(e).endswith('HTTP 404'):raise
                source_ids[lid]=set()
        for action in plan['actions']:
            if action['kind']=='calendar' and action['list'] not in plan['remove_lists'] and action['task']['id'] in source_ids[action['list']]:
                self.request('DELETE',TASK+'/lists/'+enc(action['list'])+'/tasks/'+enc(action['task']['id']))
        for lid in plan['remove_lists']:
            try:remaining=self.pages(TASK+'/lists/'+enc(lid)+'/tasks?maxResults=100&showCompleted=true&showHidden=true')
            except RuntimeError as e:
                if str(e).endswith('HTTP 404'):continue
                raise
            preserved={a['task']['id'] for i,a in enumerate(plan['actions']) if a.get('list')==lid and a.get('task') and str(i) in journal}
            if any(t['id'] not in preserved for t in remaining):raise RuntimeError('Unmigrated entries remain in source list; refusing to delete')
            self.request('DELETE',TASK+'/users/@me/lists/'+enc(lid))
        for event in plan.get('calendar_cleanup',[]):
            self.request('DELETE',CAL+'/calendars/'+enc(event['calendar'])+'/events/'+enc(event['id']))
        self.local(plan,journal)
        self.verify(plan,journal)
        self.save('result.json',{'calendar_tasks':len({(a['calendar'],a['id']) for a in plan['actions'] if a['kind'] in ('calendar','orphan','series-source')} - {(a['calendar'],a['id']) for a in plan.get('calendar_cleanup',[])}),'inbox_tasks':sum(a['kind']=='inbox' for a in plan['actions']),'removed_lists':len(plan['remove_lists']),'verified':True})
    def local(self,plan,journal):
        events={};ids={};aliases=[]
        for index,a in enumerate(plan['actions']):
            dest=journal[str(index)]
            if a.get('task'):
                old_id='task:'+a['task']['id'];old_cal='tasks:'+a['list'];new_id=dest['id'] if 'calendar' in dest else 'task:'+dest['id'];new_cal=dest.get('calendar') or 'tasks:'+dest['list']
                ids[old_id]=new_id;events[(old_cal,old_id)]=(new_cal,new_id,a);aliases.append((new_cal+'/'+new_id,a['data']['id']))
            if 'calendar' in dest:events[(dest['calendar'],dest['id'])]=(dest['calendar'],dest['id'],a)
        by_task={}
        for a in plan['actions']:
            if a['kind'] in ('calendar','orphan','series-source'):
                by_task.setdefault(a['data']['id'],(a['calendar'],a['id'],a))
        def transform(v):
            if isinstance(v,list):return [transform(x) for x in v if not (isinstance(x,dict) and x.get('id') in {'tasks:'+lid for lid in plan['remove_lists']}) and x not in ['tasks:'+lid for lid in plan['remove_lists']]]
            if isinstance(v,str):return ids.get(v,v)
            if not isinstance(v,dict):return v
            v=copy.deepcopy(v)
            if 'metadata' in v and isinstance(v['metadata'],dict):v['metadata'].pop('schedule',None)
            if 'calendar_id' in v and 'id' in v and 'details' in v:
                match=events.get((v['calendar_id'],v['id']))
                if match is None:match=by_task.get((v.get('details',{}).get('task_data') or {}).get('id'))
                v['details'].pop('scheduled',None)
                if match:
                    cid,eid,a=match;v['calendar_id']=cid;v['id']=eid;v['details']['task_data']=a['data']
                    meta=a['data']['metadata'];v['details']['projects']=meta.get('projects',[]);v['details']['labels']=meta.get('labels',[])
                    if 'body' in a:
                        v['title']=a['body'].get('summary',v['title']);v['details']['description']=a['body']['description']
                        # Preserve the authoritative Calendar interval rather than the Google Tasks due date.
                        for field in ('start','end'):
                            t=a['body'].get(field)
                            if t:
                                v[field]=t.get('dateTime') or datetime.datetime.fromisoformat(t['date']).astimezone().isoformat()
                                v['all_day']='date' in t
                v['details'].pop('scheduled',None)
            return {k:transform(x) for k,x in v.items() if k!='time_block_list'}
        db=sqlite3.connect(self.db_path,timeout=30);db.execute('BEGIN IMMEDIATE')
        try:
            for table,col,pk in [('tasks','metadata','id'),('google_task_records','event','id'),('google_entities','event','id'),('google_task_backend_state','data','task_id'),('google_operation_progress','data','operation_id'),('google_operations','payload','id'),('calendar_months','events','month'),('documents','value','key')]:
                for rowid,raw in db.execute(f'SELECT {pk},{col} FROM {table} WHERE {col} IS NOT NULL').fetchall():
                    value=json.loads(raw);result=transform(value)
                    if table=='tasks':result.pop('schedule',None)
                    db.execute(f'UPDATE {table} SET {col}=? WHERE {pk}=?',(compact(result),rowid))
            for source,tid in aliases:db.execute('INSERT INTO task_aliases(source,task_id) VALUES(?,?) ON CONFLICT(source) DO UPDATE SET task_id=excluded.task_id',(source,tid))
            for index,a in enumerate(plan['actions']):
                db.execute('INSERT INTO tasks(id,metadata) VALUES(?,?) ON CONFLICT(id) DO UPDATE SET metadata=excluded.metadata',(a['data']['id'],compact(a['data']['metadata'])))
            # Re-key cached task rows after updating their JSON.
            rows=db.execute('SELECT id,event FROM google_task_records').fetchall();db.execute('DELETE FROM google_task_records')
            for _,raw in rows:
                event=json.loads(raw);db.execute('INSERT OR REPLACE INTO google_task_records(id,event) VALUES(?,?)',(event['id'],raw))
            # Queue entity keys carry both IDs, so update their keys as well as payloads.
            for (old_cal,old_id),(cid,eid,a) in events.items():
                old=old_cal+'\n'+old_id;new=cid+'\n'+eid
                if old!=new:
                    db.execute('UPDATE google_operations SET entity=? WHERE entity=?',(new,old))
                    db.execute('UPDATE OR REPLACE google_entities SET id=? WHERE id=?',(new,old))
            assert not db.execute('PRAGMA foreign_key_check').fetchall()
            assert db.execute('PRAGMA integrity_check').fetchone()[0]=='ok'
            db.commit()
        except:db.rollback();raise
        finally:db.close()
        text=self.config_path.read_text();text=re.sub(r'^time_block_list\s*=.*\n','',text,flags=re.M)
        # Remap calendar task ordering in configuration without reserializing unrelated settings.
        for old,new in ids.items():text=text.replace('"'+old+'"','"'+new+'"')
        tmp=self.config_path.with_suffix('.migration.tmp');tmp.write_text(text);tmp.chmod(0o600);tmp.replace(self.config_path)
        self.stable_notes(plan,journal)

    def stable_notes(self,plan,journal):
        db=sqlite3.connect(self.db_path)
        note_ids={source.rsplit('/',1)[-1]:str(tid) for source,tid in db.execute('SELECT source,task_id FROM task_aliases')}
        db.close()
        for index,a in enumerate(plan['actions']):
            tid=str(a['data']['id']);dest=journal[str(index)]
            note_ids[dest['id'] if 'calendar' in dest else 'task:'+dest['id']]=tid
            if a.get('task'):note_ids['task:'+a['task']['id']]=tid
        notes_dir=pathlib.Path(self.config.get('notes',{}).get('directory','')).expanduser()
        if self.config.get('notes',{}).get('directory') and notes_dir.is_dir():
            count=0
            for path in notes_dir.rglob('*.md'):
                raw=path.read_text()
                if not raw.startswith('---\n'):continue
                end=raw.find('\n---',4)
                if end<0:continue
                front=raw[:end]
                for old,new in note_ids.items():front=front.replace('"'+old+'"','"'+new+'"').replace('id: '+old+'\n','id: '+new+'\n')
                updated=front+raw[end:]
                if updated!=raw:
                    backup=self.root/'notes-before'/path.relative_to(notes_dir);backup.parent.mkdir(parents=True,exist_ok=True)
                    if not backup.exists():shutil.copy2(path,backup)
                    path.write_text(updated);count+=1
            self.save('notes-migrated.json',{'files':count})

    def verify(self,plan,journal):
        lists=self.pages(TASK+'/users/@me/lists?maxResults=100')
        assert not any(l['id'] in plan['remove_lists'] for l in lists),'Retired list still exists'
        for entry in lists:
            tasks=self.pages(TASK+'/lists/'+enc(entry['id'])+'/tasks?maxResults=100&showCompleted=true&showHidden=true')
            assert not any(t.get('due') for t in tasks),'A dated task remains in Google Tasks'
            for task in tasks:
                _,data=footer(task.get('notes',''))
                if data:assert '\ntt-schedule: ' not in task.get('notes','')
        for i,a in enumerate(plan['actions']):
            if a['kind'] not in ('calendar','orphan','series-source'):continue
            if any(e['calendar']==a['calendar'] and e['id']==a['id'] for e in plan.get('calendar_cleanup',[])):continue
            remote=self.request('GET',CAL+'/calendars/'+enc(a['calendar'])+'/events/'+enc(a['id']))
            actual=remote['extendedProperties']['private']
            assert not any(k in actual for k in ('ttTask','ttTaskList','ttTaskLink','ttBlock','ttOrder','ttDeadline'))
            data=json.loads(''.join(actual['ttData'+str(n)] for n in range(int(actual['ttDataCount']))))
            assert data==a['data'],'Metadata changed during migration'


if __name__=='__main__':
    home=pathlib.Path.home();p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--apply',action='store_true');p.add_argument('--backup',default='.migration-backups/calendar-tasks')
    p.add_argument('--config',default=str(home/'.config/timeslay/config.toml'));p.add_argument('--database',default=str(home/'.local/share/timeslay/timeslay.sqlite3'));p.add_argument('--secrets',default=str(home/'.local/state/timeslay/secrets.json'))
    args=p.parse_args();m=Migration(args);plan=m.plan(m.snapshot())
    print(compact({'actions':len(plan['actions']),'calendar_tasks':len({(a['calendar'],a['id']) for a in plan['actions'] if a['kind'] in ('calendar','orphan','series-source')} - {(a['calendar'],a['id']) for a in plan.get('calendar_cleanup',[])}),'inbox_tasks':sum(a['kind']=='inbox' for a in plan['actions']),'lists_to_remove':len(plan['remove_lists'])}))
    if args.apply:m.apply(plan);print('Migration verified and local data updated.')
