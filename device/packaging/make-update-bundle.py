"""Assemble signed browser and one-time microSD update ZIPs from a built deb."""
from pathlib import Path
import argparse, base64, hashlib, json, subprocess, tempfile, zipfile

parser=argparse.ArgumentParser()
parser.add_argument('package', type=Path)
parser.add_argument('--output', type=Path, required=True)
parser.add_argument('--sd', action='store_true', help='Also create the microSD installer')
args=parser.parse_args()
package=args.package.resolve()
signature=package.with_suffix(package.suffix+'.sig')
raw_signature=base64.b64decode(signature.read_text().strip(),validate=True)
assert len(raw_signature)==64
public=base64.b64decode((Path(__file__).parent/'update-key.pub').read_text().strip(),validate=True)
assert len(public)==32
with tempfile.TemporaryDirectory(prefix='openstreamer-signature-') as temporary:
    key=Path(temporary)/'public.der'; sig=Path(temporary)/'signature'
    key.write_bytes(bytes.fromhex('302a300506032b6570032100')+public)
    sig.write_bytes(raw_signature)
    subprocess.run(['openssl','pkeyutl','-verify','-pubin','-keyform','DER','-inkey',str(key),
                    '-rawin','-in',str(package),'-sigfile',str(sig)],check=True)
def field(name):
    return subprocess.check_output(['dpkg-deb','-f',str(package),name],text=True).strip()
assert field('Package')=='openstreamer' and field('Architecture')=='arm64'
version=field('Version')
health=version.rsplit('-',1)[0]
short=health.split('+')[0]
digest=hashlib.sha256(package.read_bytes()).hexdigest()
args.output.mkdir(parents=True,exist_ok=True)
single=args.output/f'Openstreamer-{short}.osupd'
single.write_bytes(b'OPENSTREAMER-UPDATE/1\n'+raw_signature+package.read_bytes())
readme=f'''Openstreamer {health}

On the current release: choose Openstreamer-{short}.osupd in System > Updates,
then Install. The signature is included. Settings and recordings are kept.

You can also choose both the .deb and matching .deb.sig from this ZIP.
Install only compatible packages signed with the Openstreamer update key.
The ZIP never includes a private signing key.
'''
browser=args.output/f'Openstreamer-{short}-web-update.zip'
with zipfile.ZipFile(browser,'w',zipfile.ZIP_DEFLATED) as out:
    out.write(package,package.name); out.write(signature,signature.name)
    out.writestr('README.txt',readme)
tools=Path(__file__).parent/'sd-update'
runner=(tools/'run.sh.in').read_text().replace('@VERSION@',version).replace('@HEALTH_VERSION@',health).replace('@SHA256@',digest)
manifest=json.dumps({'version':health,'package_version':version,'sha256':digest},indent=2)
guide=f'''Update Openstreamer to {health} without a terminal

1. Shut down the Pi properly (System > General > Power off), then disconnect power.
2. Insert the microSD card into a Windows computer and extract this ZIP to the computer.
3. Run Install-update.cmd. Select the root of the microSD boot partition containing
   config.txt, cmdline.txt, and bcm2711-rpi-4-b.dtb. Confirm your selection.
4. Safely eject the microSD card, insert it into the Pi, and connect power.
5. Wait for installation and the automatic reboot. Do not disconnect power.
6. Open the web interface. System > About/Updates should show {health}.

The system, settings, and recordings are not overwritten with an image. The
temporary boot parameter is removed before installation to prevent an update
loop if an error occurs.
If installation fails, openstreamer-update/result.txt and install.log will
appear on the boot partition. The original cmdline.txt is saved in the same
directory as cmdline.before. If the Pi does not boot, use your computer to
restore the contents of cmdline.before to cmdline.txt.

For subsequent updates: System > Updates > select one signed .osupd > Install.
A .deb and matching .sig from web-update.zip are also accepted together.
Signatures are verified with the Openstreamer key. This updates the application
package; the installed kernel and OS are preserved.
'''
artifacts=[single,browser]
if args.sd:
    sd=args.output/f'Openstreamer-{short}-sd-update.zip'
    with zipfile.ZipFile(sd,'w',zipfile.ZIP_DEFLATED) as out:
        for name in ['Install-update.cmd','Install-update.ps1']: out.write(tools/name,name)
        out.writestr('README.txt',guide)
        out.write(package,'payload/package.deb');out.write(signature,'payload/package.deb.sig')
        out.writestr('payload/manifest.json',manifest)
        info=zipfile.ZipInfo('payload/run.sh');info.external_attr=0o100755<<16;info.compress_type=zipfile.ZIP_DEFLATED
        out.writestr(info,runner)
    artifacts.append(sd)
for path in artifacts:
    if path.suffix=='.zip':
        with zipfile.ZipFile(path) as check: assert check.testzip() is None
    checksum=hashlib.sha256(path.read_bytes()).hexdigest()
    path.with_suffix(path.suffix+'.sha256').write_text(f'{checksum}  {path.name}\n')
    print(path)
