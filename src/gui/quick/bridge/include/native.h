#ifndef QBT_QUICK_NATIVE_H
#define QBT_QUICK_NATIVE_H

#include <cstdint>
#include <memory>

#include <QString>

namespace Qbutt
{
    class WorkModel;
    class VariantModel;

    std::unique_ptr<WorkModel> createWorkModel();
    std::unique_ptr<VariantModel> createVariantModel();
    QString contentError(std::int32_t operation);
    void logContentError(const QString &operation, const QString &detail);
    QString formatSize(std::uint64_t bytes);
}

#endif
