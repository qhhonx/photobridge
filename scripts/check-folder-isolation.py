#!/usr/bin/env python3
"""The mobile native libraries must not link the optional folder-source engine."""
import subprocess
for target in ['aarch64-apple-ios-sim','aarch64-linux-android']:
    tree=subprocess.check_output(['cargo','tree','--locked','-p','photobridge-native','--no-default-features','--target',target,'--prefix','none'],text=True)
    assert 'photobridge-folder-source' not in tree, f'Folder module leaked into {target}'
print('Folder capability isolation passed for iOS and Android')
