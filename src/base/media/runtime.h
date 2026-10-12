/*
 * Copyright (C) 2026 qbutt contributors
 * SPDX-License-Identifier: GPL-2.0-or-later
 */

#pragma once

#include <functional>
#include <optional>

#include <QByteArray>
#include <QHash>
#include <QObject>
#include <QPointer>
#include <QProcess>
#include <QString>
#include <QTimer>

namespace Media
{
    class Runtime final : public QObject
    {
        Q_OBJECT

    public:
        enum class Error
        {
            None,
            Missing,
            CouldNotStart,
            InvalidResponse,
            IncompatibleVersion,
            IncompleteInstallation,
            Timeout
        };

        enum class Purpose
        {
            Playback,
            Download
        };

        struct Info
        {
            QString executable;
            QString runtimeDir;
            QString mpv;
            QString ffmpeg;
            QString ffprobe;
            QString ytDlp;
        };

        struct Result
        {
            Info info;
            Error error = Error::None;
        };

        using Callback = std::function<void(const Result &)>;

        static Runtime *instance();
        static QString errorMessage(Error error, Purpose purpose);

        quint64 request(QObject *context, Callback callback);
        void cancel(quint64 id);

    private:
        enum class State
        {
            Idle,
            Probing,
            Retiring
        };

        struct Pending
        {
            QPointer<QObject> context;
            Callback callback;
            std::optional<Result> ready;
            QMetaObject::Connection destroyed;
        };

        explicit Runtime(QObject *parent);
        ~Runtime() override;

        void startProbe();
        void readOutput();
        void finishProbe(int exitCode, QProcess::ExitStatus exitStatus);
        void failProbe(Error error);
        void complete(const Result &result);
        void deliver(quint64 id);
        bool hasWaitingRequests() const;

        QHash<quint64, Pending> m_requests;
        QProcess m_probe;
        QTimer m_timer;
        QByteArray m_output;
        Info m_cached;
        quint64 m_nextId = 0;
        State m_state = State::Idle;
    };
}
