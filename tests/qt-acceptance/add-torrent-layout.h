// SPDX-License-Identifier: GPL-2.0-or-later
#pragma once

class AddNewTorrentDialog;
class QJsonObject;

namespace AddTorrentLayoutAcceptance
{
    void run(AddNewTorrentDialog *dialog, const QJsonObject &spec, QJsonObject &evidence);
    void verifyRestored(AddNewTorrentDialog *dialog, QJsonObject &evidence);
}
