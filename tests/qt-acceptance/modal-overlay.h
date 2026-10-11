// SPDX-License-Identifier: GPL-2.0-or-later
#pragma once

class QJsonObject;
class QWidget;

void exerciseModalOverlay(QWidget *window, const QJsonObject &spec, QJsonObject &evidence);
