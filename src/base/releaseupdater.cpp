/*
 * Copyright (C) 2026 qbutt contributors
 * SPDX-License-Identifier: GPL-2.0-or-later
 */

#include "releaseupdater.h"

#include <algorithm>
#include <optional>

#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QNetworkReply>
#include <QNetworkRequest>
#include <QRegularExpression>
#include <QSaveFile>
#include <QStringList>

#include "global.h"
#include "version.h"

namespace
{
    constexpr qint64 MAX_ARCHIVE_SIZE = 512 * 1024 * 1024;
    const QString RELEASE_ROOT = u"https://github.com/qbutt-org/qbutt/releases/"_s;

    struct Version
    {
        QStringList core;
        QStringList prerelease;
    };

    std::optional<Version> parseVersion(const QString &text)
    {
        static const QRegularExpression pattern {
            u"^(0|[1-9][0-9]*)\\.(0|[1-9][0-9]*)\\.(0|[1-9][0-9]*)(?:-([0-9A-Za-z-]+(?:\\.[0-9A-Za-z-]+)*))?$"_s};
        const auto match = pattern.match(text);
        if ((text.size() > 128) || !match.hasMatch())
            return {};
        Version result {{match.captured(1), match.captured(2), match.captured(3)}, {}};
        if (!match.captured(4).isEmpty())
            result.prerelease = match.captured(4).split(u'.');
        for (const QString &part : result.prerelease)
        {
            if ((part.size() > 1) && part.startsWith(u'0')
                && std::all_of(part.begin(), part.end(), [](QChar c) { return c.isDigit(); }))
                return {};
        }
        return result;
    }

    int compareIdentifier(const QString &a, const QString &b)
    {
        const auto numeric = [](const QString &s)
        { return std::all_of(s.begin(), s.end(), [](QChar c) { return c.isDigit(); }); };
        const bool aNumeric = numeric(a);
        const bool bNumeric = numeric(b);
        if (aNumeric != bNumeric)
            return aNumeric ? -1 : 1;
        if (aNumeric && (a.size() != b.size()))
            return a.size() < b.size() ? -1 : 1;
        return QString::compare(a, b, Qt::CaseSensitive);
    }

    int compareVersion(const Version &a, const Version &b)
    {
        for (int i = 0; i < 3; ++i)
        {
            if (const int value = compareIdentifier(a.core[i], b.core[i]); value != 0)
                return value;
        }
        if (a.prerelease.isEmpty() != b.prerelease.isEmpty())
            return a.prerelease.isEmpty() ? 1 : -1;
        for (qsizetype i = 0; i < std::min(a.prerelease.size(), b.prerelease.size()); ++i)
        {
            if (const int value = compareIdentifier(a.prerelease[i], b.prerelease[i]); value != 0)
                return value;
        }
        return (a.prerelease.size() > b.prerelease.size()) - (a.prerelease.size() < b.prerelease.size());
    }

    bool allowedUrl(const QUrl &url)
    {
        return (url.scheme() == u"https") && url.userInfo().isEmpty() && !url.hasFragment()
            && ((url.port() == -1) || (url.port() == 443))
            && ((url.host() == u"api.github.com") || (url.host() == u"github.com")
                || (url.host() == u"release-assets.githubusercontent.com")
                || (url.host() == u"objects.githubusercontent.com"));
    }

    QByteArray assetHash(const QJsonObject &asset)
    {
        static const QRegularExpression pattern {u"^sha256:([0-9a-f]{64})$"_s};
        const auto match = pattern.match(asset.value(u"digest"_s).toString());
        return match.hasMatch() ? QByteArray::fromHex(match.captured(1).toLatin1()) : QByteArray {};
    }
}

ReleaseUpdater::ReleaseUpdater(QObject *parent)
    : QObject(parent)
    , m_network(this)
{
    m_deadline.setSingleShot(true);
    connect(&m_deadline, &QTimer::timeout, this, [this]() { fail(tr("The update request timed out.")); });
}

ReleaseUpdater::~ReleaseUpdater()
{
    stop();
}

ReleaseUpdater::State ReleaseUpdater::state() const { return m_state; }
QString ReleaseUpdater::message() const { return m_message; }
QString ReleaseUpdater::fileName() const { return m_fileName; }
QString ReleaseUpdater::savedPath() const { return m_savedPath; }

void ReleaseUpdater::setState(const State state, const QString &message)
{
    m_state = state;
    m_message = message;
    emit changed();
}

void ReleaseUpdater::stop()
{
    m_deadline.stop();
    if (m_reply)
    {
        m_reply->disconnect(this);
        m_reply->abort();
        m_reply->deleteLater();
        m_reply = nullptr;
    }
    m_file.reset(); // QSaveFile removes an uncommitted partial file.
    m_buffer.clear();
}

void ReleaseUpdater::fail(const QString &message)
{
    stop();
    setState(State::Error, message);
}

void ReleaseUpdater::cancel()
{
    if ((m_state != State::Checking) && (m_state != State::Downloading))
        return;
    stop();
    setState(State::Canceled, tr("Canceled."));
}

void ReleaseUpdater::check()
{
    stop();
    m_version.clear();
    m_savedPath.clear();
    m_fileName.clear();
    m_deadline.start(60000);
    setState(State::Checking, tr("Checking GitHub Releases…"));
    fetch(QUrl(u"https://api.github.com/repos/qbutt-org/qbutt/releases?per_page=100"_s), Request::Releases);
}

void ReleaseUpdater::fetch(const QUrl &url, const Request request, const int redirects)
{
    if (!allowedUrl(url) || (redirects > 3))
    {
        fail(tr("The release server redirected to an unexpected location."));
        return;
    }
    m_request = request;
    m_redirects = redirects;
    m_received = 0;
    m_buffer.clear();
    m_hash.reset();
    QNetworkRequest networkRequest {url};
    networkRequest.setRawHeader("User-Agent", "qbutt/" QBUTT_VERSION);
    networkRequest.setRawHeader("Accept", request == Request::Releases ? "application/vnd.github+json" : "application/octet-stream");
    networkRequest.setAttribute(QNetworkRequest::RedirectPolicyAttribute, QNetworkRequest::ManualRedirectPolicy);
    networkRequest.setTransferTimeout(30000);
    m_reply = m_network.get(networkRequest);
    connect(m_reply, &QIODevice::readyRead, this, &ReleaseUpdater::readData);
    connect(m_reply, &QNetworkReply::finished, this, &ReleaseUpdater::finishRequest);
}

void ReleaseUpdater::readData()
{
    if (!m_reply)
        return;
    const bool success = m_reply->attribute(QNetworkRequest::HttpStatusCodeAttribute).toInt() == 200;
    qint64 limit = 64 * 1024;
    if (success)
        limit = (m_request == Request::Archive) ? m_archiveSize : (2 * 1024 * 1024);
    while (m_reply && m_reply->bytesAvailable())
    {
        const QByteArray data = m_reply->read(64 * 1024);
        m_received += data.size();
        if (m_received > limit)
        {
            fail(tr("The release response exceeded its expected size."));
            return;
        }
        if (!success)
            continue;
        if (m_request == Request::Archive)
        {
            m_hash.addData(data);
            if (!m_file || (m_file->write(data) != data.size()))
            {
                fail(tr("Could not write the downloaded archive."));
                return;
            }
            emit progress(m_received, m_archiveSize);
        }
        else
            m_buffer += data;
    }
}

void ReleaseUpdater::finishRequest()
{
    readData();
    if (!m_reply)
        return;
    QNetworkReply *const reply = m_reply;
    m_reply = nullptr;
    reply->deleteLater();
    const int status = reply->attribute(QNetworkRequest::HttpStatusCodeAttribute).toInt();
    if ((status >= 300) && (status < 400))
    {
        const QUrl redirect = reply->attribute(QNetworkRequest::RedirectionTargetAttribute).toUrl();
        fetch(reply->url().resolved(redirect), m_request, m_redirects + 1);
        return;
    }
    if ((reply->error() != QNetworkReply::NoError) || (status != 200))
    {
        fail(tr("Could not retrieve the release (HTTP %1, network error %2). Try again later.")
            .arg(status).arg(static_cast<int>(reply->error())));
        return;
    }
    if (m_request == Request::Releases)
        readReleases();
    else
    {
        if ((m_received != m_archiveSize) || (m_hash.result() != m_archiveHash))
        {
            fail(tr("The download is incomplete or damaged. Please try again."));
            return;
        }
        if (!m_file->commit())
        {
            fail(tr("Could not save the download."));
            return;
        }
        stop();
        setState(State::Ready, tr("Download complete. Close qbutt and extract the archive to install the update."));
    }
}

void ReleaseUpdater::readReleases()
{
    const QJsonDocument document = QJsonDocument::fromJson(m_buffer);
    const auto current = parseVersion(QStringLiteral(QBUTT_VERSION));
    if (!current || !document.isArray() || document.array().isEmpty() || (document.array().size() > 100))
    {
        fail(tr("GitHub returned invalid release metadata."));
        return;
    }
    QJsonObject selected;
    Version highest = *current;
    bool recognized = false;
    for (const auto &value : document.array())
    {
        const QJsonObject release = value.toObject();
        if (release.value(u"draft"_s).toBool(true))
            continue;
        const QString tag = release.value(u"tag_name"_s).toString();
        const auto version = tag.startsWith(u'v') ? parseVersion(tag.mid(1)) : std::nullopt;
        if (!version || (release.value(u"html_url"_s).toString() != RELEASE_ROOT + u"tag/" + tag))
            continue;
        if (current->prerelease.isEmpty() && (!version->prerelease.isEmpty() || release.value(u"prerelease"_s).toBool()))
            continue;
        recognized = true;
        if (compareVersion(*version, highest) > 0)
        {
            highest = *version;
            selected = release;
        }
    }
    if (!recognized)
    {
        fail(tr("No qbutt releases were found. Try again later."));
        return;
    }
    if (selected.isEmpty())
    {
        stop();
        setState(State::Current, tr("qbutt %1 is up to date.").arg(QStringLiteral(QBUTT_VERSION)));
        return;
    }
    m_version = selected.value(u"tag_name"_s).toString().mid(1);
    m_fileName = u"qbutt-%1-windows-x64.zip"_s.arg(m_version);
    const QString prefix = RELEASE_ROOT + u"download/v" + m_version + u'/';
    QJsonObject archive;
    int matches = 0;
    for (const auto &value : selected.value(u"assets"_s).toArray())
    {
        const QJsonObject asset = value.toObject();
        const QString name = asset.value(u"name"_s).toString();
        if (name != m_fileName)
            continue;
        ++matches;
        if ((asset.value(u"browser_download_url"_s).toString() != prefix + name)
            || (asset.value(u"state"_s).toString() != u"uploaded"))
        {
            fail(tr("The download information is invalid. Try again later."));
            return;
        }
        archive = asset;
    }
    m_archiveSize = archive.value(u"size"_s).toInteger();
    m_archiveHash = assetHash(archive);
    if ((matches != 1) || (m_archiveSize <= 0) || (m_archiveSize > MAX_ARCHIVE_SIZE) || m_archiveHash.isEmpty())
    {
        fail(tr("The Windows download is not available. Try again later."));
        return;
    }
    m_archiveUrl = QUrl(prefix + m_fileName);
    stop();
    setState(State::Available, tr("qbutt %1 is available (%2 MiB).").arg(m_version).arg(m_archiveSize / (1024 * 1024)));
}

void ReleaseUpdater::download(const QString &destination)
{
    if ((m_state != State::Available) || destination.isEmpty())
        return;
    m_file = std::make_unique<QSaveFile>(destination);
    m_file->setDirectWriteFallback(false);
    if (!m_file->open(QIODevice::WriteOnly))
    {
        fail(tr("Could not create the download file."));
        return;
    }
    m_savedPath = destination;
    m_deadline.start(10 * 60 * 1000);
    setState(State::Downloading, tr("Downloading qbutt %1…").arg(m_version));
    fetch(m_archiveUrl, Request::Archive);
}
