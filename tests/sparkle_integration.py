"""Actual Sparkle installation against disposable apps; never opens a photo library."""
import functools, http.server, os, pathlib, plistlib, shutil, subprocess, tempfile, threading, uuid
ROOT=pathlib.Path(__file__).resolve().parents[1]
SPARKLE=ROOT/'build/dependencies/sparkle'
def run(*args, **kwargs):
    return subprocess.run([str(a) for a in args],check=True,timeout=120,**kwargs)
with tempfile.TemporaryDirectory(prefix='photobridge-update-test-') as temp:
    root=pathlib.Path(temp); serve=root/'serve';serve.mkdir()
    class Handler(http.server.SimpleHTTPRequestHandler):
        def log_message(self, *args): pass
    server=http.server.ThreadingHTTPServer(('127.0.0.1',0),functools.partial(Handler,directory=str(serve)))
    thread=threading.Thread(target=server.serve_forever,daemon=True);thread.start()
    try:
        origin=f'http://127.0.0.1:{server.server_port}'
        key=root/'key'; public=run('xcrun','swift','-module-cache-path',ROOT/'build/test-modules',ROOT/'tests/GenerateTestUpdateKey.swift',key,capture_output=True,text=True).stdout.strip()
        ident='org.photobridge.update-fixture.'+uuid.uuid4().hex
        (root/'stub.c').write_text('int main(void) { return 0; }\n')
        run('xcrun','clang',root/'stub.c','-o',root/'stub')
        apps=[]
        for label,version in [('installed','1'),('update','2')]:
            app=root/label/'Update Fixture.app'; c=app/'Contents';(c/'MacOS').mkdir(parents=True);(c/'Resources').mkdir()
            shutil.copyfile(root/'stub',c/'MacOS/Fixture');(c/'MacOS/Fixture').chmod(0o755)
            info=dict(CFBundleIdentifier=ident,CFBundleName='Update Fixture',CFBundleExecutable='Fixture',CFBundlePackageType='APPL',CFBundleVersion=version,CFBundleShortVersionString='0.0.'+version,LSMinimumSystemVersion='14.0',SUPublicEDKey=public,SUFeedURL=origin+'/appcast.xml',SURequireSignedFeed=True,SUVerifyUpdateBeforeExtraction=True,SUEnableAutomaticChecks=False,NSAppTransportSecurity={'NSAllowsLocalNetworking':True})
            (c/'Info.plist').write_bytes(plistlib.dumps(info));run('codesign','--force','--sign','-',app);apps.append(app)
        run('ditto','-c','-k','--keepParent','--norsrc',apps[1],serve/'update.zip')
        run(SPARKLE/'bin/generate_appcast','--ed-key-file',key,'--download-url-prefix',origin+'/','--maximum-deltas','0',serve)
        runner=root/'Test Driver.app'; contents=runner/'Contents';(contents/'MacOS').mkdir(parents=True);(contents/'Frameworks').mkdir()
        shutil.copytree(SPARKLE/'Sparkle.framework',contents/'Frameworks/Sparkle.framework',symlinks=True)
        (contents/'Info.plist').write_bytes(plistlib.dumps(dict(CFBundleIdentifier=ident+'.driver',CFBundleName='Update Test Driver',CFBundleExecutable='Driver',CFBundlePackageType='APPL',NSAppTransportSecurity={'NSAllowsLocalNetworking':True})))
        run('xcrun','swiftc','-parse-as-library','-module-cache-path',ROOT/'build/test-modules','-F',SPARKLE,'-framework','Sparkle','-framework','AppKit','-Xlinker','-rpath','-Xlinker','@executable_path/../Frameworks',ROOT/'tests/SparkleIntegration.swift','-o',contents/'MacOS/Driver')
        run('codesign','--force','--sign','-',runner)
        feed=serve/'appcast.xml'; original=feed.read_bytes()
        feed.write_bytes(original.replace(b'<title>',b'<title>tampered ',1))
        run(contents/'MacOS/Driver',apps[0],'--expect-rejection')
        feed.write_bytes(original)
        run(contents/'MacOS/Driver',apps[0])
        installed=plistlib.loads((apps[0]/'Contents/Info.plist').read_bytes())
        assert installed['CFBundleVersion']=='2'
        run('codesign','--verify','--deep','--strict',apps[0])
        print('PASS: corrupted feed rejected; version 1 replaced with signed version 2 in temporary directory')
    finally: server.shutdown();server.server_close()
