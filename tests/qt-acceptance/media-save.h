// SPDX-License-Identifier: GPL-2.0-or-later
#pragma once

#include <QJsonObject>
#include <QUrl>

namespace Content { class ContentView; }
namespace Media { class Player; }

void exerciseMediaSave(Content::ContentView &view, Media::Player &player, quint64 token,
    const QUrl &url, const QString &variantId, const QJsonObject &spec, QJsonObject &evidence);

void exerciseMediaRuntimeFailure(Content::ContentView &view, const QString &variantId,
    const QJsonObject &spec, QJsonObject &evidence);
