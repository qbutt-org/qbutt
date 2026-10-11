#include <algorithm>
#include <thread>

#include <windows.h>

#include <QCoreApplication>
#include <QJsonDocument>
#include <QJsonObject>
#include <QTimer>

#include "session.h"

namespace
{
    constexpr qsizetype maxRequestBytes = 16 * 1024;

    void send(const QJsonObject &response)
    {
        const QByteArray data = QJsonDocument {response}.toJson(QJsonDocument::Compact) + '\n';
        DWORD offset = 0;
        while (offset < static_cast<DWORD>(data.size()))
        {
            DWORD written = 0;
            if (!WriteFile(GetStdHandle(STD_OUTPUT_HANDLE), data.constData() + offset,
                static_cast<DWORD>(data.size()) - offset, &written, nullptr) || !written)
                break;
            offset += written;
        }
    }
}

int WINAPI wWinMain(HINSTANCE, HINSTANCE, PWSTR arguments, int)
{
    if (QString::fromWCharArray(arguments).trimmed() != "--stdio")
        return 2;
    int argc = 1;
    char program[] = "qbutt-webview";
    char *argv[] = {program, nullptr};
    QCoreApplication application {argc, argv};
    if (GetFileType(GetStdHandle(STD_INPUT_HANDLE)) != FILE_TYPE_PIPE
        || GetFileType(GetStdHandle(STD_OUTPUT_HANDLE)) != FILE_TYPE_PIPE)
        return 2;

    // Overrides can select another profile, engine or command line before the
    // native options are applied. This private process owns all of them.
    for (const wchar_t *name : {L"WEBVIEW2_BROWSER_EXECUTABLE_FOLDER", L"WEBVIEW2_USER_DATA_FOLDER",
        L"WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS", L"WEBVIEW2_RELEASE_CHANNEL_PREFERENCE",
        L"WEBVIEW2_CHANNEL_SEARCH_KIND", L"WEBVIEW2_RELEASE_CHANNELS"})
        SetEnvironmentVariableW(name, nullptr);

    const HRESULT apartment = CoInitializeEx(nullptr, COINIT_APARTMENTTHREADED);
    if (FAILED(apartment))
        return 2;
    const HANDLE stopping = CreateEventW(nullptr, TRUE, FALSE, nullptr);
    if (!stopping)
    {
        CoUninitialize();
        return 2;
    }

    // The helper and its descendants form one lifetime. A killed parent or a
    // broken stdio channel cannot leave a hidden browser process behind.
    const HANDLE job = CreateJobObjectW(nullptr, nullptr);
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION limits {};
    limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    if (!job || !SetInformationJobObject(job, JobObjectExtendedLimitInformation, &limits, sizeof(limits))
        || !AssignProcessToJobObject(job, GetCurrentProcess()))
    {
        if (job)
            CloseHandle(job);
        CoUninitialize();
        return 2;
    }

    WebSession session {[&application](const QJsonObject &response)
    {
        send(response);
        application.quit();
    }};
    QTimer inputDeadline;
    inputDeadline.setSingleShot(true);
    inputDeadline.start(3000);
    QObject::connect(&inputDeadline, &QTimer::timeout, &session, [&session] { session.cancel(); });
    std::thread input {[&]
    {
        QByteArray pending;
        while (WaitForSingleObject(stopping, 0) == WAIT_TIMEOUT)
        {
            char bytes[4096];
            DWORD count = 0;
            DWORD available = 0;
            const HANDLE pipe = GetStdHandle(STD_INPUT_HANDLE);
            if (!PeekNamedPipe(pipe, nullptr, 0, nullptr, &available, nullptr))
                break;
            if (!available)
            {
                WaitForSingleObject(stopping, 25);
                continue;
            }
            // This thread is the sole reader. Read only bytes already present
            // so shutdown cannot race an uncancellable future blocking read.
            if (!ReadFile(pipe, bytes, std::min(available, static_cast<DWORD>(sizeof(bytes))), &count, nullptr) || !count)
                break;
            pending.append(bytes, count);
            if (pending.size() > maxRequestBytes)
                break;
            qsizetype newline = 0;
            while ((newline = pending.indexOf('\n')) >= 0)
            {
                const QByteArray line = pending.left(newline);
                pending.remove(0, newline + 1);
                QMetaObject::invokeMethod(&session, [&session, &inputDeadline, line]
                {
                    const QJsonDocument document = QJsonDocument::fromJson(line);
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
                }, Qt::QueuedConnection);
            }
        }
        QMetaObject::invokeMethod(&session, [&session] { session.cancel(); }, Qt::QueuedConnection);
    }};

    const int result = application.exec();
    SetEvent(stopping);
    input.join();
    CloseHandle(stopping);
    CoUninitialize();
    // Keep the non-inheritable job handle until process teardown. Closing it
    // here would terminate this member process before its normal exit status.
    // Windows closes it on exit and kills any surviving owned descendants.
    return result;
}
