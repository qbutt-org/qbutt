// SPDX-License-Identifier: GPL-2.0-or-later
#pragma once

class MainWindow;
class QJsonObject;

void exerciseReaderFlow(MainWindow *window, const QJsonObject &spec, QJsonObject &evidence);
