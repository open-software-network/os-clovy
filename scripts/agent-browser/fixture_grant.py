"""Synthetic Linux fixture grant backed by a real profile and kernel lease."""
import fcntl
import os
import uuid


def persistent_fixture_grant(profile):
    profile.mkdir(mode=0o700)
    lease = (profile / '.clovy-lease').open('x+')
    fcntl.flock(lease.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
    root = profile.parent
    root_stat = root.stat(follow_symlinks=False)
    profile_stat = profile.stat(follow_symlinks=False)
    pid = os.getpid()
    with open(f'/proc/{pid}/stat', encoding='utf8') as stat:
        tail = stat.read().rsplit(')', 1)[1].split()
    birth = int(tail[19])
    return ({'version':1,'profile':str(profile),'root':str(root),'nonce':str(uuid.uuid4()),
             'rootIdentity':[root_stat.st_dev,root_stat.st_ino],
             'profileIdentity':[profile_stat.st_dev,profile_stat.st_ino],
             'parentPid':pid,'parentBirth':birth,'persistent':True}, lease)
