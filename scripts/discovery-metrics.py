#!/usr/bin/env python3
"""Print a dated GitHub discovery snapshot; no files or remote state are changed."""
import datetime
import json
import subprocess

REPO = 'hulryung-uo/anima-client'

def api(path):
    result = subprocess.run(['gh', 'api', '--paginate', path], capture_output=True, text=True)
    if result.returncode:
        return {'unavailable': True, 'reason': 'GitHub request failed; check authentication and repository permissions.'}
    try:
        return json.loads(result.stdout)
    except json.JSONDecodeError:
        # Paginated endpoints return adjacent JSON arrays in older gh versions.
        decoder, remaining, records = json.JSONDecoder(), result.stdout.strip(), []
        while remaining:
            page, end = decoder.raw_decode(remaining)
            records.extend(page)
            remaining = remaining[end:].lstrip()
        return records

repo = api(f'repos/{REPO}')
releases = api(f'repos/{REPO}/releases?per_page=100')
report = {
    'captured_at': datetime.datetime.now(datetime.timezone.utc).isoformat(),
    'repository': REPO,
    'public': {key: repo.get(key) for key in ('stargazers_count', 'forks_count', 'open_issues_count')},
    'releases': [
        {'tag': release['tag_name'], 'published_at': release['published_at'],
         'assets': [{'name': a['name'], 'downloads': a['download_count']} for a in release['assets']]}
        for release in releases if not release['draft']
    ] if isinstance(releases, list) else releases,
    'views_14_days': api(f'repos/{REPO}/traffic/views'),
    'clones_14_days': api(f'repos/{REPO}/traffic/clones'),
    'referrers_14_days': api(f'repos/{REPO}/traffic/popular/referrers'),
    'limits': 'Downloads are asset request counts, not installations or unique users. Traffic can include automation. No website analytics are collected.',
}
if repo.get('unavailable'):
    report['public'] = repo
print(json.dumps(report, indent=2, ensure_ascii=False))
