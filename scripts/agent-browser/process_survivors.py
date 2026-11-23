"""Run-owned Linux process identities for the private browser proof."""
import os
from pathlib import Path


def snapshot():
    processes = {}
    for entry in Path('/proc').iterdir():
        if not entry.name.isdigit():
            continue
        try:
            tail = (entry / 'stat').read_text().rsplit(')', 1)[1].split()
            processes[int(entry.name)] = {
                'state': tail[0], 'parent': int(tail[1]), 'group': int(tail[2]),
                'birth': int(tail[19]),
            }
        except (OSError, ValueError, IndexError):
            continue
    return processes


def descendants(processes, root, known):
    owned = {root}
    owned.update(pid for pid, birth in known.items()
                 if pid in processes and processes[pid]['birth'] == birth)
    changed = True
    while changed:
        before = len(owned)
        owned.update(pid for pid, info in processes.items() if info['parent'] in owned)
        changed = len(owned) != before
    return owned


def same_image(pid, image):
    try:
        actual = os.stat(f'/proc/{pid}/exe')
        return (actual.st_dev, actual.st_ino) == image
    except OSError:
        return False


def invalidate_reused_groups(processes, groups):
    for group, birth in list(groups.items()):
        leader = processes.get(group)
        if leader is not None and leader['birth'] != birth:
            del groups[group]


def survivors(processes, known, groups):
    valid_groups = dict(groups)
    invalidate_reused_groups(processes, valid_groups)
    return sorted(pid for pid, info in processes.items()
                  if info['state'] not in ('Z', 'X') and
                  (known.get(pid) == info['birth'] or
                   (info['group'] in valid_groups and info['birth'] >= valid_groups[info['group']])))
