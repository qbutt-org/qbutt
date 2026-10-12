/*
 * Copyright (C) 2026 qbutt contributors
 * SPDX-License-Identifier: GPL-2.0-or-later
 */

#include "player.h"

#include <utility>

#include <QCoreApplication>
#include <QDir>
#include <QFileInfo>
#include <QJsonDocument>
#include <QRegularExpression>
#include <QSettings>
#include <QVersionNumber>

#include "base/global.h"
#include "streamserver.h"

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
}

void Player::startPending()
{
    if (!m_pending.url.isValid() || (m_process.state() != QProcess::NotRunning)
        || (m_runtimeProbeState != RuntimeProbeState::Idle))
        return;

    if (!m_executable.isEmpty())
    {
        startPrepared();
        return;
    }

    const QString executable = playerExecutable();
    if (executable.isEmpty() || !QFileInfo(executable).isFile())
    {
        failPreparation(tr("Install qplay to watch videos."));
        return;
    }

    m_runtimeOutput.clear();
    m_runtimeProbeState = RuntimeProbeState::Probing;
    m_runtimeTimer.start();
    m_runtimeProbe.start(executable, {u"--runtime-info-json"_s});
    if (m_runtimeProbeState == RuntimeProbeState::Probing)
        emit preparationChanged(tr("Preparing player…"));
}

void Player::cancelPreparation()
{
    m_runtimeTimer.stop();
    if (m_runtimeProbeState != RuntimeProbeState::Probing)
        return;

    m_runtimeOutput.clear();
    m_runtimeProbeState = RuntimeProbeState::Retiring;
    if (m_runtimeProbe.state() == QProcess::NotRunning)
        m_runtimeProbeState = RuntimeProbeState::Idle;
    else
        m_runtimeProbe.kill();
}

void Player::failPreparation(const QString &message)
{
    Playback pending = std::exchange(m_pending, {});
    cancelPreparation();
    StreamServer::instance()->closeStream(pending.url);
    emit preparationChanged({});
    finishLaunch(pending);
    emit error(message);
}

void Player::readRuntimeInfo()
{
    const QByteArray output = m_runtimeProbe.read(MAX_RUNTIME_OUTPUT + 1);
    if (m_runtimeProbeState != RuntimeProbeState::Probing)
        return;

    if (output.size() > MAX_RUNTIME_OUTPUT - m_runtimeOutput.size())
    {
        failPreparation(tr("qplay returned an invalid response. Reinstall qplay."));
        return;
    }
    m_runtimeOutput.append(output);
}

void Player::finishRuntimeProbe(const int exitCode, const QProcess::ExitStatus exitStatus)
{
    m_runtimeTimer.stop();
    const RuntimeProbeState state = std::exchange(m_runtimeProbeState, RuntimeProbeState::Idle);
    QByteArray output = std::exchange(m_runtimeOutput, {});
    output.append(m_runtimeProbe.read(MAX_RUNTIME_OUTPUT + 1));
    if (state == RuntimeProbeState::Retiring)
    {
        startPending();
        return;
    }
    if (state != RuntimeProbeState::Probing)
        return;

    if (output.size() > MAX_RUNTIME_OUTPUT)
    {
        failPreparation(tr("qplay returned an invalid response. Reinstall qplay."));
        return;
    }

    if ((exitStatus != QProcess::NormalExit) || (exitCode != 0))
    {
        failPreparation(tr("qplay could not start. Reinstall qplay."));
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
        failPreparation(tr("Install qplay %1 or newer to watch videos.").arg(minimumVersion.toString()));
        return;
    }

    bool valid = QFileInfo(info.value(u"runtimeDir"_s).toString()).isAbsolute();
    const QJsonObject tools = info.value(u"tools"_s).toObject();
    for (const QString &name : {u"mpv"_s, u"ffmpeg"_s, u"ffprobe"_s, u"ytDlp"_s})
    {
        const QFileInfo file(tools.value(name).toString());
        valid = valid && file.isAbsolute() && file.isFile() && !file.isSymLink();
    }
    if (!valid)
    {
        failPreparation(tr("The qplay installation is incomplete. Reinstall qplay."));
        return;
    }

    m_executable = m_runtimeProbe.program();
    emit preparationChanged({});
    startPending();
}
