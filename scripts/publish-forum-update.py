#!/usr/bin/env python3
"""Publish release or feature notes to UO Tavern. Dry run unless --publish is set."""
import argparse
import json
import os
from pathlib import Path
import re
import subprocess
import urllib.error
import urllib.parse
import urllib.request

ROOT = Path(__file__).resolve().parent.parent
REPOSITORY = 'hulryung-uo/anima-client'
ENDPOINT = 'https://www.uotavern.com/forum/api/client-updates'

def git(*args):
    return subprocess.check_output(['git', *args], cwd=ROOT, text=True).strip()

def read_note(path):
    target = (ROOT / path).resolve()
    if not target.is_relative_to(ROOT / 'docs/updates') or target.suffix != '.json':
        raise ValueError('Feature notes must be JSON files under docs/updates/.')
    note = json.loads(target.read_text())
    if not isinstance(note, dict):
        raise ValueError('Expected an update object.')
    return note

def images_for(note, revision):
    images=[]
    for image in note.get('images', []):
        alt=image.get('alt', '').strip()
        if not alt or len(alt)>200: raise ValueError('Every screenshot needs a description (up to 200 characters).')
        if 'path' in image:
            path=(ROOT / image['path']).resolve()
            if not path.is_relative_to(ROOT / 'docs') or not path.is_file(): raise ValueError('Screenshot paths must be existing files under docs/.')
            if path.suffix.lower() not in ('.png','.jpg','.jpeg','.webp'): raise ValueError('Use PNG, JPEG, or WebP screenshots.')
            url=f'https://raw.githubusercontent.com/{REPOSITORY}/{revision}/'+urllib.parse.quote(path.relative_to(ROOT).as_posix(),safe='/')
        else:
            url=image.get('url','')
        if urllib.parse.urlsplit(url).scheme!='https': raise ValueError('Screenshot URLs must use HTTPS.')
        images.append({'url':url,'alt':alt})
    if len(images)>5: raise ValueError('Use up to five screenshots per update.')
    return images

def release_payload(tag, note=None):
    note=note or {}
    command=['gh','release','view',tag,'--repo',REPOSITORY,'--json','tagName,name,body,isDraft,isPrerelease,publishedAt,url']
    release=json.loads(subprocess.check_output(command,text=True))
    if release['isDraft'] or release['isPrerelease']: raise ValueError('Only published, stable releases are announced automatically.')
    revision=git('rev-parse','HEAD')
    images=images_for(note,revision)
    content=note.get('content') or release.get('body') or 'A new Anima client release is available. See the release page for details.'
    content=f"Released {release['publishedAt'][:10]}.\n\n"+content
    limit=min(7900,9500-sum(len(i['url'])+len(i['alt'])+12 for i in images)-len(release['url']))
    if len(content)>limit: content=content[:limit-100].rsplit('\n\n',1)[0]+'\n\nContinue in the full release notes linked below.'
    return {'key':'release:'+release['tagName'],'title':(note.get('title') or release['name'] or f"Anima {release['tagName']}")[:180],'content':content,'sourceUrl':release['url'],'images':images}

def feature_payload(note, path):
    slug=note.get('slug','')
    if not re.fullmatch(r'[a-zA-Z0-9][a-zA-Z0-9._-]{0,99}',slug): raise ValueError('A feature needs a stable slug.')
    title=note.get('title','').strip();content=note.get('content','').strip()
    if not 1<=len(title)<=180 or not 1<=len(content)<=8000: raise ValueError('Provide a title (1–180 characters) and content (1–8000 characters).')
    revision=git('rev-parse','HEAD')
    return {'key':'feature:'+slug,'title':title,'content':content,'sourceUrl':f'https://github.com/{REPOSITORY}/blob/{revision}/'+urllib.parse.quote(Path(path).as_posix(),safe='/'),'images':images_for(note,revision)}

def from_note(path):
    note=read_note(path)
    if note.get('kind')=='release':return release_payload(note['tag'],note)
    if note.get('kind')=='feature':return feature_payload(note,path)
    raise ValueError('Note kind must be release or feature.')

def changed_notes(before, after):
    if not re.fullmatch('[0-9a-f]{40}',before) or not re.fullmatch('[0-9a-f]{40}',after):raise ValueError('Expected full git commit IDs.')
    if before=='0'*40: return []  # Do not publish the entire archive on a new branch.
    paths=git('diff','--name-only','--diff-filter=AM',before,after,'--','docs/updates/*.json').splitlines()
    return [path for path in paths if (ROOT/path).is_file()]

def send(payload,publish):
    if not publish:
        print(json.dumps(payload,ensure_ascii=False,indent=2));return
    token=os.environ.get('UOTAVERN_PUBLISH_TOKEN')
    if not token:raise ValueError('UOTAVERN_PUBLISH_TOKEN is not configured.')
    request=urllib.request.Request(ENDPOINT,data=json.dumps(payload).encode(),headers={'Content-Type':'application/json','Authorization':'Bearer '+token},method='POST')
    try:
        with urllib.request.urlopen(request,timeout=45) as response:result=json.load(response)
    except urllib.error.HTTPError as error:
        # Server responses contain validation messages, never the publisher token.
        raise ValueError(f'Forum returned HTTP {error.code}: '+error.read(2000).decode(errors='replace')) from None
    print(json.dumps({key:result.get(key) for key in ('action','url')},ensure_ascii=False))

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    choice=parser.add_mutually_exclusive_group(required=True)
    choice.add_argument('--release');choice.add_argument('--note');choice.add_argument('--changed-before')
    parser.add_argument('--changed-after');parser.add_argument('--publish',action='store_true')
    args=parser.parse_args()
    if args.release:
        note_path=ROOT/'docs/updates'/f'{args.release}.json'
        note=read_note(note_path) if note_path.is_file() else {}
        payloads=[release_payload(args.release,note)]
    elif args.note:payloads=[from_note(args.note)]
    else:
        if not args.changed_after:parser.error('--changed-after is required')
        payloads=[from_note(path) for path in changed_notes(args.changed_before,args.changed_after)]
    for payload in payloads:send(payload,args.publish)
    if not payloads:print('No feature notes to publish.')

if __name__=='__main__':
    try:main()
    except (ValueError,KeyError,subprocess.CalledProcessError,urllib.error.URLError) as error:
        raise SystemExit(str(error))
