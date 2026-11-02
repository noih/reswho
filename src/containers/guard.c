#include <sys/event.h>
#include <sys/wait.h>
#include <sys/types.h>
#include <unistd.h>
#include <signal.h>
#include <errno.h>
#include <fcntl.h>
#include <stdlib.h>

int reswho_guard(int lifeline) {
    if (dup2(lifeline, 3) < 0 || fcntl(3, F_SETFD, FD_CLOEXEC) < 0 || setsid() < 0)
        _exit(125);
    int limit = getdtablesize();
    for (int fd = 4; fd < limit; ++fd) close(fd);
    int gate[2];
    if (pipe(gate) < 0) _exit(125);
    pid_t child = fork();
    if (child < 0) _exit(125);
    if (child == 0) {
        close(3);
        close(gate[1]);
        char ready;
        ssize_t received;
        do { received = read(gate[0], &ready, 1); } while (received < 0 && errno == EINTR);
        close(gate[0]);
        if (received != 1) _exit(125);
        return 0;
    }
    close(gate[0]);
    int grouped = setpgid(child, child) == 0;
    int queue = kqueue();
    int monitored = 0;
    if (grouped && queue >= 0) {
        struct kevent changes[2];
        EV_SET(&changes[0], 3, EVFILT_READ, EV_ADD, 0, 0, NULL);
        EV_SET(&changes[1], child, EVFILT_PROC, EV_ADD, NOTE_EXIT, 0, NULL);
        int result;
        do { result = kevent(queue, changes, 2, NULL, 0, NULL); } while (result < 0 && errno == EINTR);
        monitored = result == 0;
    }
    if (monitored) {
        char ready = 1;
        ssize_t sent;
        do { sent = write(gate[1], &ready, 1); } while (sent < 0 && errno == EINTR);
    }
    close(gate[1]);
    close(0);
    close(1);
    close(2);
    if (monitored) {
        struct kevent event;
        int result;
        do { result = kevent(queue, NULL, 0, &event, 1, NULL); } while (result < 0 && errno == EINTR);
    }
    kill(grouped ? -child : child, SIGKILL);
    int status = 0;
    pid_t waited;
    do { waited = waitpid(child, &status, 0); } while (waited < 0 && errno == EINTR);
    if (queue >= 0) close(queue);
    close(3);
    if (waited < 0 || !monitored) _exit(125);
    if (WIFSIGNALED(status)) {
        int sig = WTERMSIG(status);
        signal(sig, SIG_DFL);
        sigset_t mask;
        sigemptyset(&mask);
        sigaddset(&mask, sig);
        sigprocmask(SIG_UNBLOCK, &mask, NULL);
        kill(getpid(), sig);
        _exit(128 + sig);
    }
    _exit(WEXITSTATUS(status));
}
