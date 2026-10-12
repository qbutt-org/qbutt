// SPDX-License-Identifier: GPL-2.0-or-later
#include "workfiltermodel.h"

void Content::WorkFilterModel::setSourceModel(QAbstractItemModel *source)
{
    setFilterRole(source ? source->roleNames().key("category", -1) : -1);
    QSortFilterProxyModel::setSourceModel(source);
}

QString Content::WorkFilterModel::idAt(const int row) const
{
    return data(index(row, 0), roleNames().key("workId", -1)).toString();
}

int Content::WorkFilterModel::indexOfId(const QString &id) const
{
    const int role = roleNames().key("workId", -1);
    for (int row = 0; row < rowCount(); ++row)
    {
        if (data(index(row, 0), role).toString() == id)
            return row;
    }
    return -1;
}
