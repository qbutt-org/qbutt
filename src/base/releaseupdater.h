/*
 * Copyright (C) 2026 qbutt contributors
 * SPDX-License-Identifier: GPL-2.0-or-later
 */

#pragma once

#include <memory>

#include <QCryptographicHash>
#include <QNetworkAccessManager>
#include <QObject>
#include <QPointer>
#include <QTimer>
#include <QUrl>

class QNetworkReply;
class QSaveFile;

// Downloads an entire portable release. Never installs code or changes a profile.
class ReleaseUpdater final : public QObject
{
    Q_OBJECT
    Q_DISABLE_COPY_MOVE(ReleaseUpdater)

public:
    enum class State { Idle, Checking, Available, Current, Downloading, Ready, Canceled, Error };
    Q_ENUM(State)

    explicit ReleaseUpdater(QObject *parent = nullptr);
    ~ReleaseUpdater() override;

    State state() const;
    QString message() const;
    QString fileName() const;
    QString savedPath() const;
    void check();
    void download(const QString &destination);
    void cancel();

signals:
    void changed();
    void progress(qint64 received, qint64 total);

private:
    enum class Request { Releases, Checksums, Signature, Archive };
    void fetch(const QUrl &url, Request request, int redirects = 0);
    void readData();
    void finishRequest();
    void readReleases();
    void setState(State state, const QString &message);
    void fail(const QString &message);
    void stop();

    QNetworkAccessManager m_network;
    QPointer<QNetworkReply> m_reply;
    QTimer m_deadline;
    std::unique_ptr<QSaveFile> m_file;
    QCryptographicHash m_hash {QCryptographicHash::Sha256};
    State m_state = State::Idle;
    Request m_request = Request::Releases;
    QString m_message;
    QString m_version;
    QString m_fileName;
    QString m_savedPath;
    QUrl m_archiveUrl;
    QUrl m_checksumsUrl;
    QByteArray m_archiveHash;
    QByteArray m_checksumsHash;
    QByteArray m_checksums;
    QByteArray m_buffer;
    qint64 m_archiveSize = 0;
    qint64 m_received = 0;
    int m_redirects = 0;
};
