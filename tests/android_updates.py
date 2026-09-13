"""Real PackageManager update checks in an isolated, disposable Android package."""
from pathlib import Path
import argparse, os, shutil, subprocess
p=argparse.ArgumentParser();p.add_argument('--serial',required=True);args=p.parse_args()
r=Path(__file__).resolve().parents[1]; d=r/'build/update-fixtures'; d.mkdir(parents=True,exist_ok=True)
sdk=Path(os.environ['ANDROID_HOME']); adb=str(sdk/'platform-tools/adb'); package='app.photobridge.updatefixture'
env=dict(os.environ)
key=d/'foreign.keystore'
if not key.exists():
 subprocess.run(['keytool','-genkeypair','-keystore',str(key),'-storepass','fixture-only','-keypass','fixture-only','-alias','photobridge','-keyalg','RSA','-validity','30','-dname','CN=Disposable update fixture'],check=True,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
env.update(ANDROID_KEYSTORE_FILE=str(key),ANDROID_KEYSTORE_PASSWORD='fixture-only')
base=['./gradlew','-PvalidationApp','-PupdateFixture']
for build,kind,name in [(1,'Debug','host.apk'),(2,'Debug','next.apk'),(2,'Release','other-key.apk')]:
 subprocess.run(base+[f'-PfixtureBuild={build}','assemble'+kind]+(['assembleDebugAndroidTest'] if build==1 else []),cwd=r/'apps/android',env=env,check=True,stdout=subprocess.DEVNULL)
 shutil.copy(r/f'apps/android/app/build/outputs/apk/{kind.lower()}/app-{kind.lower()}.apk',d/name)
 if build==1: shutil.copy(r/'apps/android/app/build/outputs/apk/androidTest/debug/app-debug-androidTest.apk',d/'tests.apk')
def device(*items,**kw): return subprocess.run([adb,'-s',args.serial,*items],check=True,**kw)
try:
 device('install','-r',str(d/'host.apk'))
 device('install','-r',str(d/'tests.apk'))
 device('shell','run-as',package,'mkdir','-p','files/update-fixtures')
 for name in ['next.apk','other-key.apk']:
  remote='/data/local/tmp/photobridge-update-'+name
  device('push',str(d/name),remote,stdout=subprocess.DEVNULL)
  device('shell','chmod','644',remote)
  device('shell','run-as',package,'cp',remote,'files/update-fixtures/'+name)
  device('shell','rm',remote)
 output=device('shell','am','instrument','-w','-e','mode','app_updates',package+'.test/app.photobridge.ReceiverInstrumentation',capture_output=True,text=True).stdout
 print(output)
 assert 'PASS: genuine higher build accepted' in output
finally:
 device('uninstall',package+'.test',stdout=subprocess.DEVNULL)
 device('uninstall',package,stdout=subprocess.DEVNULL)
