// SPDX-License-Identifier: GPL-2.0-or-later
#pragma once

#include <QSortFilterProxyModel>
#include <QtQmlIntegration/qqmlintegration.h>

namespace Content
{
    class WorkFilterModel : public QSortFilterProxyModel
    {
        Q_OBJECT
        QML_ELEMENT

    public:
        void setSourceModel(QAbstractItemModel *source) override;

        Q_INVOKABLE QString idAt(int row) const;
        Q_INVOKABLE int indexOfId(const QString &id) const;
    };
}
