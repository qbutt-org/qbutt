/*
 * Copyright (C) 2026 qbutt contributors
 * SPDX-License-Identifier: GPL-2.0-or-later
 */

#pragma once

class QJsonObject;
class QObject;

namespace SubtitlesPlaybackAcceptance
{
    void run(QObject *errorReceiver, const QJsonObject &spec, QJsonObject &evidence);
}
