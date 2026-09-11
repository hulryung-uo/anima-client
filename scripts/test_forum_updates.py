import importlib.util
import json
from pathlib import Path
import unittest
from unittest.mock import patch
spec=importlib.util.spec_from_file_location('publisher',Path(__file__).with_name('publish-forum-update.py'))
publisher=importlib.util.module_from_spec(spec);spec.loader.exec_module(publisher)
class PublishingTests(unittest.TestCase):
 def test_feature_identity_and_pinned_screenshots(self):
  note={'kind':'feature','slug':'night-lights','title':'A torch','content':'Carry a light.','images':[{'path':'docs/img/night.png','alt':'A torch in Britain'}]}
  with patch.object(publisher,'git',return_value='a'*40):payload=publisher.feature_payload(note,'docs/updates/night-lights.json')
  self.assertEqual(payload['key'],'feature:night-lights')
  self.assertIn('/'+'a'*40+'/',payload['images'][0]['url'])
 def test_note_cannot_read_outside_updates(self):
  with self.assertRaises(ValueError):publisher.read_note('../../.env.local')
 def test_image_path_cannot_escape_repository(self):
  with self.assertRaises(ValueError):publisher.images_for({'images':[{'path':'../uotavern/.env.local','alt':'x'}]},'main')
 def test_image_requires_description_and_https(self):
  for image in [{'url':'https://example.com/a.png','alt':''},{'url':'javascript:alert(1)','alt':'x'}]:
   with self.assertRaises(ValueError):publisher.images_for({'images':[image]},'main')
 def test_draft_and_prerelease_never_publish(self):
  for release in [{'isDraft':True,'isPrerelease':False},{'isDraft':False,'isPrerelease':True}]:
   with patch.object(publisher.subprocess,'check_output',return_value=json.dumps(release)):
    with self.assertRaises(ValueError):publisher.release_payload('v-test')
 def test_changed_notes_require_commit_ids(self):
  with self.assertRaises(ValueError):publisher.changed_notes('$(cat .env)','HEAD')
 def test_dry_run_never_sends_a_request(self):
  with patch.object(publisher.urllib.request,'urlopen') as call,patch('builtins.print'):
   publisher.send({'key':'feature:test'},False);call.assert_not_called()
if __name__=='__main__':unittest.main()
