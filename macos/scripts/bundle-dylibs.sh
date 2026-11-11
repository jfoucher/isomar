#!/usr/bin/env python3
"""Makes an Isomar.app that runs without Homebrew: copies the Homebrew libraries the binary
needs (and theirs) into Contents/Frameworks, points every reference at @rpath, and signs the
result ad hoc (a changed arm64 binary must be signed again).

Usage: [SIGN_IDENTITY="Developer ID Application: ..."] bundle-dylibs.sh Isomar.app

Signs with SIGN_IDENTITY (hardened runtime, with the app's entitlements) or ad hoc when it is not set.
"""
import os, shutil, subprocess, sys

IDENTITY = os.environ.get("SIGN_IDENTITY", "-")
LIB_DIRS = [os.path.expanduser("~/dev/isomar-deps/universal/lib")] + os.environ.get("ISOMAR_LIB_DIRS", "").split(":")
ENTITLEMENTS = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "Isomar", "Isomar.entitlements")

def sign(path, app=False):
    cmd = ["codesign", "--force", "--sign", IDENTITY]
    if IDENTITY != "-":
        cmd += ["--options", "runtime", "--timestamp"]
        if app:
            cmd += ["--entitlements", ENTITLEMENTS]
    run(*cmd, path)

SYSTEM = ("/usr/lib/", "/System/")

def run(*cmd):
    return subprocess.run(cmd, check=True, capture_output=True, text=True).stdout

def deps(path):
    # Every slice: on an Intel Mac a plain `otool -L` lists only the x86_64 one, and 0.7.0 shipped arm64
    # slices that named the build folder. Each slice is listed under a header line; references start with a tab.
    lines = [l for l in run("otool", "-arch", "all", "-L", path).splitlines() if l.startswith("\t")]
    return list(dict.fromkeys(l.split(" (compatibility")[0].strip() for l in lines))

def check(fw, exe):
    """Every slice of every file must find its libraries in the system or in Frameworks."""
    bad = []
    for f in [exe] + [os.path.join(fw, n) for n in sorted(os.listdir(fw))]:
        for d in deps(f):
            if d.startswith(SYSTEM):
                continue
            if not (d.startswith("@rpath/") and os.path.exists(os.path.join(fw, d[7:]))):
                bad.append(f"{os.path.basename(f)}: {d}")
    if bad:
        sys.exit("libraries found outside the app:\n  " + "\n  ".join(bad))

def main(app):
    exe = os.path.join(app, "Contents", "MacOS", "Isomar")
    fw = os.path.join(app, "Contents", "Frameworks")
    # A bundle that was bundled before (Xcode runs this after every build, and does not always relink) holds
    # a binary that already names @rpath/<lib>: the earlier copies are where those are found again.
    prev = fw + ".prev"
    shutil.rmtree(prev, ignore_errors=True)
    if os.path.isdir(fw):
        os.rename(fw, prev)
    os.makedirs(fw)
    LIB_DIRS.append(prev)
    origin = {exe: exe}                 # bundle file -> where it came from
    queue = [exe]
    copied = {}                          # real source path -> bundle file
    while queue:
        f = queue.pop()
        src_dir = os.path.dirname(origin[f])
        for d in deps(f):
            if d.startswith(SYSTEM):
                continue
            if d.startswith("@loader_path/"):
                real = os.path.realpath(os.path.join(src_dir, d[len("@loader_path/"):]))
            elif d.startswith("@rpath/"):
                # The universal libraries are named @rpath/<name>; find the file in the folders they were built into.
                real = next((os.path.join(dd, d[7:]) for dd in LIB_DIRS if os.path.exists(os.path.join(dd, d[7:]))), None)
                if real is None:
                    continue
                real = os.path.realpath(real)
            elif d.startswith("@executable_path/"):
                continue
            else:
                # A slice naming one architecture's build folder: the universal library of that name, when
                # there is one, so that the thin file does not replace it in Frameworks.
                uni = os.path.join(LIB_DIRS[0], os.path.basename(d))
                real = os.path.realpath(uni if os.path.exists(uni) else d)
            if not os.path.exists(real):
                sys.exit(f"cannot find {d} needed by {f}")
            name = os.path.basename(real)
            if real not in copied:
                dest = os.path.join(fw, name)
                shutil.copy2(real, dest)
                os.chmod(dest, 0o755)
                run("install_name_tool", "-id", "@rpath/" + name, dest)
                copied[real] = dest
                origin[dest] = real
                queue.append(dest)
            if not d.startswith("@rpath/"):
                run("install_name_tool", "-change", d, "@rpath/" + name, f)
    rpaths = run("otool", "-l", exe)
    if "@executable_path/../Frameworks" not in rpaths:
        run("install_name_tool", "-add_rpath", "@executable_path/../Frameworks", exe)
    shutil.rmtree(prev, ignore_errors=True)
    check(fw, exe)
    for dest in copied.values():
        sign(dest)
    sign(app, app=True)
    print(f"bundled {len(copied)} libraries into {fw}")

main(sys.argv[1])
