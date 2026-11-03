// SPDX-License-Identifier: GPL-2.0-or-later
#include <cerrno>
#include <csignal>

#include <fcntl.h>
#include <sys/prctl.h>
#include <sys/stat.h>
#include <unistd.h>

#include <QApplication>
#include <QByteArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QSocketNotifier>
#include <QTimer>

#include "session_linux.h"

namespace
{
    void terminateGroup(int)
    {
        kill(0, SIGKILL);
    }

    bool pipeDescriptor(const int descriptor)
    {
        struct stat status {};
        return fstat(descriptor, &status) == 0 && (S_ISFIFO(status.st_mode) || S_ISSOCK(status.st_mode));
    }
}

int main(int argc, char *argv[])
{
    if (argc != 2 || QByteArray(argv[1]) != "--stdio" || !pipeDescriptor(STDIN_FILENO)
        || !pipeDescriptor(STDOUT_FILENO) || setpgid(0, 0) != 0)
    {
        return 2;
    }
    const pid_t parent = getppid();
    umask(0077);
    std::signal(SIGTERM, terminateGroup);
    std::signal(SIGINT, terminateGroup);
    std::signal(SIGPIPE, SIG_IGN);
    if (parent == 1 || prctl(PR_SET_PDEATHSIG, SIGTERM) != 0 || getppid() != parent)
        return 2;

    for (const char *name : {"QTWEBENGINE_REMOTE_DEBUGGING", "QTWEBENGINE_DISABLE_SANDBOX",
        "QTWEBENGINEPROCESS_PATH", "QTWEBENGINE_RESOURCES_PATH", "QTWEBENGINE_LOCALES_PATH"})
    {
        qunsetenv(name);
    }
    qputenv("QTWEBENGINE_CHROMIUM_FLAGS", "--disable-quic --proxy-bypass-list=<-loopback> "
        "--force-webrtc-ip-handling-policy=disable_non_proxied_udp");
    QApplication application(argc, argv);
    application.setQuitOnLastWindowClosed(false);

    WebSession session([&application](const QJsonObject &response)
    {
        const QByteArray bytes = QJsonDocument(response).toJson(QJsonDocument::Compact) + '\n';
        qsizetype offset = 0;
        while (offset < bytes.size())
        {
            const ssize_t count = write(STDOUT_FILENO, bytes.constData() + offset, bytes.size() - offset);
            if (count < 0 && errno == EINTR)
                continue;
            if (count <= 0)
                break;
            offset += count;
        }
        application.quit();
    });
    QTimer inputDeadline;
    inputDeadline.setSingleShot(true);
    inputDeadline.start(3000);
    QObject::connect(&inputDeadline, &QTimer::timeout, &session, [&session] { session.cancel(); });

    const int flags = fcntl(STDIN_FILENO, F_GETFL);
    if (flags < 0 || fcntl(STDIN_FILENO, F_SETFL, flags | O_NONBLOCK) != 0)
        return 2;
    QSocketNotifier input(STDIN_FILENO, QSocketNotifier::Read);
    QByteArray pending;
    QObject::connect(&input, &QSocketNotifier::activated, &session, [&]
    {
        char buffer[4096];
        const ssize_t count = read(STDIN_FILENO, buffer, sizeof(buffer));
        if (count < 0 && (errno == EINTR || errno == EAGAIN))
            return;
        if (count <= 0)
        {
            input.setEnabled(false);
            session.cancel();
            return;
        }
        pending.append(buffer, count);
        if (pending.size() > 16 * 1024)
        {
            input.setEnabled(false);
            session.cancel();
            return;
        }
        qsizetype newline = 0;
        while ((newline = pending.indexOf('\n')) >= 0)
        {
            const QJsonDocument document = QJsonDocument::fromJson(pending.left(newline));
            pending.remove(0, newline + 1);
            if (!document.isObject())
            {
                session.cancel();
                return;
            }
            const QJsonObject request = document.object();
            if (request.value("method").toString() == "cancel")
            {
                if (request.value("id").toString() == session.id())
                    session.cancel();
            }
            else
            {
                inputDeadline.stop();
                session.start(request);
            }
        }
    });
    return application.exec();
}
