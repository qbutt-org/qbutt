// SPDX-License-Identifier: GPL-2.0-or-later
#include "browsercapture.h"

#include <utility>

#include <QJsonArray>

#include "base/content/client.h"
#include "base/global.h"
#include "base/media/player.h"

Content::BrowserCapture::BrowserCapture(QObject *parent)
    : QObject(parent)
{
    connect(Client::instance(), &Client::eventReceived, this, &BrowserCapture::handleEvent);
}

Content::BrowserCapture::~BrowserCapture()
{
    Client::instance()->cancel(m_request);
}

void Content::BrowserCapture::open(const QUrl &url)
{
    cancel();
    if ((url.scheme() != u"qbutt") || (url.host() != u"capture"))
    {
        emit error(tr("This shared link is not supported by this version."));
        return;
    }
    m_request = Client::instance()->request(u"openLink"_s, {{u"url"_s, url.toString(QUrl::FullyEncoded)}});
    emit preparationChanged(tr("Opening qcatch video…"));
}

void Content::BrowserCapture::retry()
{
    if (m_variant.isEmpty())
        return;

    if (m_downloadRetry)
    {
        if (!requestDownload(m_work, m_variant))
            emit error(tr("Could not open this qcatch video."));
        return;
    }

    Client::instance()->cancel(m_request);
    m_request = Client::instance()->request(u"resolve"_s, {{u"work"_s, m_work}, {u"variant"_s, m_variant}});
    emit preparationChanged(tr("Opening qcatch video…"));
}

void Content::BrowserCapture::cancel()
{
    if (!m_pendingCaptureId.isEmpty())
    {
        m_pendingCaptureId.clear();
        emit downloadCancelled();
    }
    Client::instance()->cancel(std::exchange(m_request, 0));
    m_work = {};
    m_variant = {};
    m_downloadRetry = false;
    emit preparationChanged({});
}

bool Content::BrowserCapture::requestPlaybackDownload(const QString &identity)
{
    if (identity.isEmpty() || identity != m_playback.value(u"identity"_s).toString())
        return false;

    m_work = m_playback.value(u"work"_s).toObject();
    m_variant = m_playback.value(u"variant"_s).toObject();
    m_downloadRetry = true;
    return requestDownload(m_work, m_variant);
}

bool Content::BrowserCapture::requestDownload(const QJsonObject &work, const QJsonObject &variant)
{
    if (!m_pendingCaptureId.isEmpty()
        || variant.value(u"sourceId"_s).toString() != u"link"
        || variant.value(u"transport"_s).toString() != u"video"
        || variant.value(u"url"_s).toString() != work.value(u"url"_s).toString())
    {
        return false;
    }
    const QString prefix = work.value(u"id"_s).toString() + u":capture:"_s;
    const QString variantId = variant.value(u"id"_s).toString();
    if (prefix == u":capture:"_s || !variantId.startsWith(prefix))
        return false;

    const QUrl page(variant.value(u"url"_s).toString());
    const QString title = work.value(u"title"_s).toString();
    const QString captureId = variantId.mid(prefix.size());
    if (!page.isValid() || title.isEmpty() || captureId.isEmpty())
        return false;

    m_pendingCaptureId = captureId;
    emit preparationChanged(tr("Preparing download…"));
    emit downloadRequested(page, title, captureId);
    return true;
}

void Content::BrowserCapture::finishDownload(const QString &captureId, const QString &reason)
{
    if (captureId != m_pendingCaptureId)
        return;
    m_pendingCaptureId.clear();
    emit preparationChanged({});
    if (reason == u"browser_required")
    {
        m_downloadRetry = true;
        emit browserRequired();
        return;
    }
    m_downloadRetry = false;
    if (reason.isEmpty())
        emit completed();
    else if (reason == u"unsupported")
        emit error(tr("This video cannot be saved as one MP4 file."));
    else if (reason == u"capture")
        emit error(tr("This qcatch selection expired. Choose the video again in qcatch."));
    else
        emit error(tr("The download could not be queued."));
}

void Content::BrowserCapture::handleEvent(const qint64 id, const QString &event, const QJsonValue &data)
{
    if (!m_request || (id != m_request))
        return;

    if (event == u"details")
    {
        m_work = data.toObject();
        const QString action = m_work.take(u"captureAction"_s).toString();
        const QJsonArray variants = m_work.value(u"variants"_s).toArray();
        if ((variants.size() != 1) || ((action != u"watch") && (action != u"download")))
        {
            cancel();
            emit error(tr("Could not open this qcatch video."));
            return;
        }
        m_variant = variants.first().toObject();
        if (action == u"download")
        {
            m_request = 0;
            m_downloadRetry = true;
            if (!requestDownload(m_work, m_variant))
            {
                emit preparationChanged({});
                emit error(tr("Could not open this qcatch video."));
            }
        }
        else
        {
            retry();
        }
    }
    else if (event == u"resolved")
    {
        m_request = 0;
        emit preparationChanged({});
        play(data.toObject());
        emit completed();
    }
    else if (event == u"error")
    {
        m_request = 0;
        emit preparationChanged({});
        const QJsonObject failure = data.toObject();
        if (failure.value(u"code"_s).toString() == u"browser_required")
            emit browserRequired();
        else
            emit error(failure.value(u"message"_s).toString());
    }
}

void Content::BrowserCapture::play(const QJsonObject &resolved)
{
    if (resolved.value(u"kind"_s).toString() != u"video")
    {
        emit error(tr("Could not open this qcatch video."));
        return;
    }

    Media::PlaybackOptions options;
    options.audioUrl = QUrl(resolved.value(u"audioUrl"_s).toString());
    options.proxyUrl = QUrl(resolved.value(u"proxyUrl"_s).toString());
    const QJsonObject audioTrack = resolved.value(u"audioTrack"_s).toObject();
    options.audioTrack.title = audioTrack.value(u"title"_s).toString();
    options.audioTrack.type = audioTrack.value(u"type"_s).toString();
    options.audioTrack.language = audioTrack.value(u"language"_s).toString();
    options.audioTrack.codec = audioTrack.value(u"codec"_s).toString();
    options.audioTrack.channels = audioTrack.value(u"channels"_s).toInt();
    options.contentKind = u"movie"_s;
    options.identity = u"video:"_s + m_variant.value(u"sourceId"_s).toString()
        + u':' + m_variant.value(u"id"_s).toString();
    const QJsonObject headers = resolved.value(u"headers"_s).toObject();
    for (auto it = headers.constBegin(); it != headers.constEnd(); ++it)
        options.headers.insert(it.key(), it.value().toString());
    for (const QJsonValue &subtitle : resolved.value(u"subtitles"_s).toArray())
        options.subtitles.append(QUrl(subtitle.toObject().value(u"url"_s).toString()));

    if (Media::Player::instance()->playUrl(QUrl(resolved.value(u"url"_s).toString()),
        m_work.value(u"title"_s).toString(), options))
    {
        m_playback = {{u"identity"_s, options.identity}, {u"work"_s, m_work}, {u"variant"_s, m_variant}};
    }
}
