// SPDX-License-Identifier: GPL-2.0-or-later
#include "peer-process.h"

#include <stdexcept>

#include <QByteArray>
#include <QFile>

#ifdef Q_OS_WIN
#include <QScopeGuard>
#include <qt_windows.h>
#endif

using namespace Qt::Literals::StringLiterals;

PeerProcess::State PeerProcess::state(const qint64 pid)
{
    if (pid <= 0)
        throw std::runtime_error("Peer process identity is missing");

#ifdef Q_OS_WIN
    HANDLE process = OpenProcess(SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION, FALSE, static_cast<DWORD>(pid));
    if (!process)
    {
        if (GetLastError() == ERROR_INVALID_PARAMETER)
            return {};
        throw std::runtime_error("Peer process state is unavailable");
    }
    const auto close = qScopeGuard([&] { CloseHandle(process); });
    FILETIME creation, exit, kernel, user;
    if (!GetProcessTimes(process, &creation, &exit, &kernel, &user))
        throw std::runtime_error("Peer creation time is unavailable");
    const DWORD result = WaitForSingleObject(process, 0);
    if ((result != WAIT_TIMEOUT) && (result != WAIT_OBJECT_0))
        throw std::runtime_error("Peer process wait failed");
    return {result == WAIT_TIMEOUT, (quint64(creation.dwHighDateTime) << 32) | creation.dwLowDateTime};
#elif defined(Q_OS_LINUX)
    QFile stat(u"/proc/%1/stat"_s.arg(pid));
    if (!stat.open(QIODevice::ReadOnly))
    {
        if (!stat.exists())
            return {};
        throw std::runtime_error(stat.errorString().toStdString());
    }
    const QByteArray record = stat.read(4096);
    const qsizetype nameEnd = record.lastIndexOf(')');
    if (nameEnd < 0)
        throw std::runtime_error("Peer process status is invalid");
    const auto fields = record.sliced(nameEnd + 2).simplified().split(' ');
    if (fields.size() < 20)
        throw std::runtime_error("Peer process status is incomplete");
    bool valid = false;
    const quint64 createdAt = fields[19].toULongLong(&valid);
    if (!valid || !createdAt)
        throw std::runtime_error("Peer creation time is invalid");
    return {(fields[0] != "Z") && (fields[0] != "X"), createdAt};
#else
#error Peer process fixtures require a native process-state implementation
#endif
}
