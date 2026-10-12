/*
 * Copyright (C) 2026 qbutt contributors
 * SPDX-License-Identifier: GPL-2.0-or-later
 */

#include "runtime.h"

#include <utility>

#include <QCoreApplication>
#include <QDir>
#include <QFileInfo>
#include <QJsonDocument>
#include <QJsonObject>
#include <QJsonValue>
#include <QRegularExpression>
#include <QSettings>
#include <QVersionNumber>

#include "base/global.h"

using namespace Media;

namespace
{
    constexpr qsizetype MAX_RUNTIME_OUTPUT = 64 * 1024;

    QString playerExecutable()
    {
        if (qEnvironmentVariable("QBUTT_LAB_PATHS") == u"1")
        {
            const QString executable = qEnvironmentVariable("QBUTT_LAB_QPLAY");
            return QFileInfo(executable).isAbsolute() ? executable : QString {};
        }

        const QDir portable(QCoreApplication::applicationDirPath() + u"/qplay"_s);
        if (portable.exists(u"qplay.portable"_s))
            return portable.filePath(u"qplay.exe"_s);

#ifdef Q_OS_WIN
        const QSettings installation(u"HKEY_CURRENT_USER\\Software\\qbutt-org\\qplay"_s, QSettings::Registry64Format);
        const QString directory = installation.value(u"InstallDir"_s).toString();
        if (QFileInfo(directory).isAbsolute())
            return QDir(directory).filePath(u"qplay.exe"_s);
#endif
        return {};
    }

    bool validFile(const QString &path)
    {
        const QFileInfo file(path);
        return file.isAbsolute() && file.isFile() && !file.isSymLink();
    }

    bool validInfo(const Runtime::Info &info)
    {
        return QFileInfo(info.runtimeDir).isAbsolute()
            && validFile(info.executable) && validFile(info.mpv)
            && validFile(info.ffmpeg) && validFile(info.ffprobe) && validFile(info.ytDlp);
    }
}

Runtime *Runtime::instance()
{
    static QPointer<Runtime> instance;
    if (!instance)
        instance = new Runtime(QCoreApplication::instance());
    return instance;
}

Runtime::Runtime(QObject *parent)
    : QObject(parent)
{
    m_probe.setStandardErrorFile(QProcess::nullDevice());
    m_timer.setSingleShot(true);
    m_timer.setInterval(30000);

    connect(&m_timer, &QTimer::timeout, this, [this] { failProbe(Error::Timeout); });
    connect(&m_probe, &QProcess::readyReadStandardOutput, this, &Runtime::readOutput);
    connect(&m_probe, &QProcess::finished, this, &Runtime::finishProbe);
    connect(&m_probe, &QProcess::errorOccurred, this, [this](const QProcess::ProcessError error)
    {
        if (error == QProcess::FailedToStart)
            failProbe(Error::CouldNotStart);
    });
}

Runtime::~Runtime()
{
    m_timer.stop();
    m_requests.clear();
    m_probe.disconnect(this);
    if (m_probe.state() != QProcess::NotRunning)
    {
        m_probe.kill();
        m_probe.waitForFinished(1000);
    }
}

quint64 Runtime::request(QObject *context, Callback callback)
{
    if (!context)
        return 0;

    const quint64 id = ++m_nextId;
    Pending pending;
    pending.context = context;
    pending.callback = std::move(callback);
    pending.destroyed = connect(context, &QObject::destroyed, this, [this, id] { cancel(id); });
    m_requests.insert(id, std::move(pending));

    if (!m_cached.executable.isEmpty() && validInfo(m_cached))
    {
        m_requests[id].ready = Result {m_cached};
        QTimer::singleShot(0, this, [this, id] { deliver(id); });
    }
    else
    {
        m_cached = {};
        startProbe();
    }
    return id;
}

void Runtime::cancel(const quint64 id)
{
    const auto it = m_requests.find(id);
    if (it == m_requests.end())
        return;

    disconnect(it->destroyed);
    m_requests.erase(it);
    if ((m_state == State::Probing) && !hasWaitingRequests())
    {
        m_state = State::Retiring;
        m_timer.stop();
        m_output.clear();
        if (m_probe.state() == QProcess::NotRunning)
            m_state = State::Idle;
        else
            m_probe.kill();
    }
}

void Runtime::startProbe()
{
    if ((m_state != State::Idle) || !hasWaitingRequests())
        return;

    const QString executable = playerExecutable();
    if (!validFile(executable))
    {
        complete({{}, Error::Missing});
        return;
    }

    m_output.clear();
    m_state = State::Probing;
    m_timer.start();
    m_probe.start(executable, {u"--runtime-info-json"_s});
}

void Runtime::readOutput()
{
    const QByteArray output = m_probe.read(MAX_RUNTIME_OUTPUT + 1);
    if (m_state != State::Probing)
        return;

    if (output.size() > MAX_RUNTIME_OUTPUT - m_output.size())
    {
        failProbe(Error::InvalidResponse);
        return;
    }
    m_output.append(output);
}

void Runtime::finishProbe(const int exitCode, const QProcess::ExitStatus exitStatus)
{
    m_timer.stop();
    const State state = m_state;
    m_state = State::Idle;
    if (state == State::Retiring)
    {
        m_output.clear();
        startProbe();
        return;
    }
    if (state != State::Probing)
        return;

    QByteArray output = std::move(m_output);
    m_output.clear();
    output.append(m_probe.read(MAX_RUNTIME_OUTPUT + 1));
    if (output.size() > MAX_RUNTIME_OUTPUT)
    {
        complete({{}, Error::InvalidResponse});
        return;
    }
    if ((exitStatus != QProcess::NormalExit) || (exitCode != 0))
    {
        complete({{}, Error::CouldNotStart});
        return;
    }

    const QJsonObject info = QJsonDocument::fromJson(output).object();
    const QString version = info.value(u"version"_s).toString();
    static const QRegularExpression versionPattern(u"^[0-9]+\\.[0-9]+\\.[0-9]+$"_s);
    const QVersionNumber actualVersion = QVersionNumber::fromString(version);
    const QVersionNumber minimumVersion = QVersionNumber::fromString(QStringLiteral(QBUTT_QPLAY_MINIMUM_VERSION));
    const QJsonValue protocol = info.value(u"protocolVersion"_s);
    if ((info.value(u"product"_s).toString() != u"qplay") || !protocol.isDouble()
        || (protocol.toDouble() != QBUTT_QPLAY_PROTOCOL_VERSION) || !versionPattern.match(version).hasMatch()
        || (actualVersion.segmentCount() != 3) || (actualVersion < minimumVersion))
    {
        complete({{}, Error::IncompatibleVersion});
        return;
    }

    const QJsonObject tools = info.value(u"tools"_s).toObject();
    Info validated {
        m_probe.program(), info.value(u"runtimeDir"_s).toString(),
        tools.value(u"mpv"_s).toString(), tools.value(u"ffmpeg"_s).toString(),
        tools.value(u"ffprobe"_s).toString(), tools.value(u"ytDlp"_s).toString()
    };
    if (!validInfo(validated))
    {
        complete({{}, Error::IncompleteInstallation});
        return;
    }

    m_cached = validated;
    complete({std::move(validated)});
}

void Runtime::failProbe(const Error error)
{
    if (m_state != State::Probing)
        return;

    m_state = State::Retiring;
    m_timer.stop();
    m_output.clear();
    complete({{}, error});
    if (m_probe.state() == QProcess::NotRunning)
        m_state = State::Idle;
    else
        m_probe.kill();
}

void Runtime::complete(const Result &result)
{
    for (auto it = m_requests.begin(); it != m_requests.end(); ++it)
    {
        if (it->ready)
            continue;

        it->ready = result;
        const quint64 id = it.key();
        QTimer::singleShot(0, this, [this, id] { deliver(id); });
    }
}

void Runtime::deliver(const quint64 id)
{
    const auto it = m_requests.find(id);
    if ((it == m_requests.end()) || !it->ready)
        return;

    Pending pending = std::move(*it);
    m_requests.erase(it);
    disconnect(pending.destroyed);
    if (pending.context)
        pending.callback(*pending.ready);
}

bool Runtime::hasWaitingRequests() const
{
    for (const Pending &pending : m_requests)
    {
        if (!pending.ready && pending.context)
            return true;
    }
    return false;
}

QString Runtime::errorMessage(const Error error, const Purpose purpose)
{
    switch (error)
    {
    case Error::Missing:
        return purpose == Purpose::Playback ? tr("Install qplay to watch videos.")
            : tr("Install qplay to download this video.");
    case Error::CouldNotStart:
        return tr("qplay could not start. Reinstall qplay.");
    case Error::InvalidResponse:
        return tr("qplay returned an invalid response. Reinstall qplay.");
    case Error::IncompatibleVersion:
        return purpose == Purpose::Playback
            ? tr("Install qplay %1 or newer to watch videos.").arg(QStringLiteral(QBUTT_QPLAY_MINIMUM_VERSION))
            : tr("Install qplay %1 or newer to download this video.").arg(QStringLiteral(QBUTT_QPLAY_MINIMUM_VERSION));
    case Error::IncompleteInstallation:
        return tr("The qplay installation is incomplete. Reinstall qplay.");
    case Error::Timeout:
        return purpose == Purpose::Playback ? tr("qplay did not respond. Try opening the video again.")
            : tr("qplay did not respond. Try downloading the video again.");
    case Error::None:
        return {};
    }
    Q_UNREACHABLE();
}
