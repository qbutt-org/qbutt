using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;

public static class DxgiGpuInfo
{
    [StructLayout(LayoutKind.Sequential)]
    public struct Luid
    {
        public uint LowPart;
        public int HighPart;
    }

    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    public struct DxgiAdapterDesc
    {
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 128)]
        public string Description;
        public uint VendorId;
        public uint DeviceId;
        public uint SubSysId;
        public uint Revision;
        public UIntPtr DedicatedVideoMemory;
        public UIntPtr DedicatedSystemMemory;
        public UIntPtr SharedSystemMemory;
        public Luid AdapterLuid;
    }

    [ComImport, InterfaceType(ComInterfaceType.InterfaceIsIUnknown), Guid("2411e7e1-12ac-4ccf-bd14-9798e8534dc0")]
    private interface IDXGIAdapter
    {
        [PreserveSig] int SetPrivateData(ref Guid name, uint dataSize, IntPtr data);
        [PreserveSig] int SetPrivateDataInterface(ref Guid name, IntPtr unknown);
        [PreserveSig] int GetPrivateData(ref Guid name, ref uint dataSize, IntPtr data);
        [PreserveSig] int GetParent(ref Guid riid, out IntPtr parent);
        [PreserveSig] int EnumOutputs(uint output, out IntPtr outputPtr);
        [PreserveSig] int GetDesc(out DxgiAdapterDesc desc);
        [PreserveSig] int CheckInterfaceSupport(ref Guid interfaceName, out long umdVersion);
    }

    [ComImport, InterfaceType(ComInterfaceType.InterfaceIsIUnknown), Guid("7b7166ec-21c7-44ae-b21a-c9ae321ae369")]
    private interface IDXGIFactory
    {
        [PreserveSig] int SetPrivateData(ref Guid name, uint dataSize, IntPtr data);
        [PreserveSig] int SetPrivateDataInterface(ref Guid name, IntPtr unknown);
        [PreserveSig] int GetPrivateData(ref Guid name, ref uint dataSize, IntPtr data);
        [PreserveSig] int GetParent(ref Guid riid, out IntPtr parent);
        [PreserveSig] int EnumAdapters(uint adapterIndex, out IDXGIAdapter adapter);
        [PreserveSig] int MakeWindowAssociation(IntPtr windowHandle, uint flags);
        [PreserveSig] int GetWindowAssociation(out IntPtr windowHandle);
        [PreserveSig] int CreateSwapChain(IntPtr device, IntPtr desc, out IntPtr swapChain);
        [PreserveSig] int CreateSoftwareAdapter(IntPtr module, out IDXGIAdapter adapter);
    }

    public sealed class AdapterInfo
    {
        public string Description;
        public uint VendorId;
        public uint DeviceId;
        public uint SubSysId;
        public uint Revision;
        public ulong DedicatedVideoMemory;
        public ulong DedicatedSystemMemory;
        public ulong SharedSystemMemory;
    }

    [DllImport("dxgi.dll", PreserveSig = true)]
    private static extern int CreateDXGIFactory(ref Guid riid, [MarshalAs(UnmanagedType.Interface)] out IDXGIFactory factory);

    public static AdapterInfo[] GetAdapters()
    {
        Guid factoryGuid = new Guid("7b7166ec-21c7-44ae-b21a-c9ae321ae369");
        IDXGIFactory factory = null;
        int hr = CreateDXGIFactory(ref factoryGuid, out factory);
        if (hr < 0) {
            Marshal.ThrowExceptionForHR(hr);
        }
        if (factory == null) {
            throw new InvalidOperationException("CreateDXGIFactory returned a null factory.");
        }

        var adapters = new List<AdapterInfo>();
        try {
            for (uint index = 0; ; index++) {
                IDXGIAdapter adapter = null;
                hr = factory.EnumAdapters(index, out adapter);
                if ((uint)hr == 0x887A0002) {
                    break;
                }
                if (hr < 0) {
                    Marshal.ThrowExceptionForHR(hr);
                }
                if (adapter == null) {
                    throw new InvalidOperationException("EnumAdapters returned a null adapter for index " + index + ".");
                }

                try {
                    DxgiAdapterDesc desc;
                    hr = adapter.GetDesc(out desc);
                    if (hr >= 0) {
                        adapters.Add(new AdapterInfo {
                            Description = desc.Description,
                            VendorId = desc.VendorId,
                            DeviceId = desc.DeviceId,
                            SubSysId = desc.SubSysId,
                            Revision = desc.Revision,
                            DedicatedVideoMemory = desc.DedicatedVideoMemory.ToUInt64(),
                            DedicatedSystemMemory = desc.DedicatedSystemMemory.ToUInt64(),
                            SharedSystemMemory = desc.SharedSystemMemory.ToUInt64()
                        });
                    }
                } finally {
                    if (adapter != null) {
                        Marshal.ReleaseComObject(adapter);
                    }
                }
            }
        } finally {
            if (factory != null) {
                Marshal.ReleaseComObject(factory);
            }
        }

        return adapters.ToArray();
    }
}
