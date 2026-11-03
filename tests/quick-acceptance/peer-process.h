// SPDX-License-Identifier: GPL-2.0-or-later
#pragma once

#include <QtGlobal>

namespace PeerProcess
{
    struct State
    {
        bool running = false;
        quint64 createdAt = 0;
    };

    State state(qint64 pid);
}
