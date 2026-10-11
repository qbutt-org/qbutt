// SPDX-License-Identifier: GPL-2.0-or-later
#pragma once

class Application;
class QJsonObject;

namespace QuickHandoffAcceptance
{
    void run(Application &application, const QJsonObject &spec, QJsonObject &evidence);
}
