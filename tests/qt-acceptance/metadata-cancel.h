// SPDX-License-Identifier: GPL-2.0-or-later
#pragma once

class QJsonObject;
class Application;

void exerciseMetadataCancel(const QJsonObject &spec, QJsonObject &evidence);
void exerciseTorrentData(Application &application, const QJsonObject &spec, QJsonObject &evidence);
