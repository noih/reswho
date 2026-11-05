#include <libproc.h>
#include <stdint.h>
#include <string.h>

int reswho_vm_probe(int pid, char paths[64][1024], int *limited) {
    struct proc_fdinfo fds[64];
    int bytes = proc_pidinfo(pid, PROC_PIDLISTFDS, 0, fds, sizeof(fds));
    if (bytes < 0 || bytes % sizeof(fds[0]) != 0) return -1;
    int count = bytes / sizeof(fds[0]);
    if (count > 64) count = 64;
    *limited = count == 64;
    int found = 0;
    for (int i = 0; i < count; i++) {
        if (fds[i].proc_fdtype != PROX_FDTYPE_VNODE) continue;
        struct vnode_fdinfowithpath info;
        memset(&info, 0, sizeof(info));
        if (proc_pidfdinfo(pid, fds[i].proc_fd, PROC_PIDFDVNODEPATHINFO, &info, sizeof(info)) != sizeof(info)) continue;
        if (!memchr(info.pvip.vip_path, '\0', sizeof(info.pvip.vip_path))) continue;
        memcpy(paths[found++], info.pvip.vip_path, sizeof(info.pvip.vip_path));
    }
    return found;
}
