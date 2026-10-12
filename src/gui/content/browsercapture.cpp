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

    Client::instance()->cancel(m_request);
    m_request = Client::instance()->request(u"resolve"_s, {{u"work"_s, m_work}, {u"variant"_s, m_variant}});
    emit preparationChanged(tr("Opening qcatch video…"));
}

void Content::BrowserCapture::cancel()
{
    Client::instance()->cancel(std::exchange(m_request, 0));
    m_work = {};
    m_variant = {};
    emit preparationChanged({});
}

QJsonObject Content::BrowserCapture::downloadParameters(const QJsonObject &params) const
{
    QJsonObject request = params;
    const QString identity = params.value(u"options"_s).toObject().value(u"identity"_s).toString();
    if (!identity.isEmpty() && (identity == m_playback.value(u"identity"_s).toString()))
    {
        request.insert(u"work"_s, m_playback.value(u"work"_s));
        request.insert(u"variant"_s, m_playback.value(u"variant"_s));
    }
    return request;
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
            emit preparationChanged({});
            emit downloadRequested({{u"work"_s, m_work}, {u"variant"_s, m_variant}});
            emit completed();
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
