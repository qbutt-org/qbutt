// SPDX-License-Identifier: GPL-2.0-or-later
#pragma once

#include <QJsonArray>
#include <QJsonObject>

namespace GroupEntry
{
    inline QJsonObject single(const QJsonObject &work)
    {
        const QJsonObject reference = work.value("reference").toObject();
        return {
            {"anchor", reference},
            {"references", QJsonArray {reference}},
            {"publications", QJsonArray {work}}
        };
    }

    inline QJsonObject anchorWork(const QJsonObject &entry)
    {
        const QJsonObject group = entry.value("group").toObject();
        const QJsonObject anchor = group.value("anchor").toObject();
        for (const QJsonValue &value : group.value("publications").toArray())
        {
            const QJsonObject work = value.toObject();
            if (work.value("reference").toObject() == anchor)
                return work;
        }
        return {};
    }

    inline void replaceAnchorWork(QJsonObject &entry, const QJsonObject &replacement)
    {
        QJsonObject group = entry.value("group").toObject();
        QJsonArray publications = group.value("publications").toArray();
        const QJsonObject anchor = group.value("anchor").toObject();
        for (qsizetype index = 0; index < publications.size(); ++index)
        {
            if (publications[index].toObject().value("reference").toObject() == anchor)
            {
                publications[index] = replacement;
                break;
            }
        }
        group.insert("publications", publications);
        entry.insert("group", group);
    }
}
