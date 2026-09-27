use jxl_winthumb::JXLWICBitmapDecoder;
use windows::Win32::Graphics::Imaging::*;
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance, CoInitialize};
use windows::Win32::UI::Shell::SHCreateMemStream;

#[test]
fn basic() {
    unsafe { CoInitialize(None) }.ok().expect("CoInitialize");

    let mem = std::fs::read("tests/alien.jxl").expect("Read the test file");
    let stream = unsafe { SHCreateMemStream(Some(&mem[..])) }.expect("Create an IStream");
    let decoder: IWICBitmapDecoder = JXLWICBitmapDecoder::default().into();
    unsafe { decoder.Initialize(&stream, WICDecodeOptions(0)) }.expect("Initialize the decoder");
    let frame = unsafe { decoder.GetFrame(0) }.expect("Get the first frame");
    let source = unsafe { WICConvertBitmapSource(&GUID_WICPixelFormat32bppPRGBA, &frame) }
        .expect("Create a bitmap source");

    let factory: IWICImagingFactory =
        unsafe { CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER) }
            .expect("Create a factory");
    let bitmap = unsafe { factory.CreateBitmapFromSource(&source, WICBitmapCacheOnDemand) }
        .expect("Create a bitmap");

    let mut width = 0u32;
    let mut height = 0u32;
    unsafe { bitmap.GetSize(&mut width, &mut height).expect("GetSize") };
    assert_eq!(width, 1024, "width");
    assert_eq!(height, 1024, "height");

    let mut pixels: Vec<u8> = vec![0; 1024 * 1024 * 4];
    unsafe {
        bitmap.CopyPixels(
            &WICRect {
                X: 0,
                Y: 0,
                Width: 1024,
                Height: 1024,
            },
            1024 * 4,
            &mut pixels,
        )
    }
    .expect("Copy pixels");
    assert_eq!(pixels[0], 0, "red");
    assert_eq!(pixels[1], 6, "green");
    assert_eq!(pixels[2], 0, "blue");
    assert_eq!(pixels[3], 255, "alpha");
}

#[test]
fn test_property_store() {
    use jxl_winthumb::JXLPropertyStore;
    use windows::Win32::Foundation::PROPERTYKEY;
    use windows::Win32::UI::Shell::PropertiesSystem::{IInitializeWithStream, IPropertyStore};
    use windows::core::{GUID, Interface};

    let mem = std::fs::read("tests/alien.jxl").expect("Read the test file");
    let stream = unsafe { SHCreateMemStream(Some(&mem[..])) }.expect("Create an IStream");

    let prop_init: IInitializeWithStream = JXLPropertyStore::default().into();
    unsafe { prop_init.Initialize(&stream, 0) }.expect("Initialize property store");

    let prop_store: IPropertyStore = prop_init.cast().expect("Cast to IPropertyStore");

    let count = unsafe { prop_store.GetCount() }.expect("GetCount");
    assert_eq!(
        count, 3,
        "Expected 3 properties (width, height, dimensions)"
    );

    let psguid_imagesummaryinformation = GUID::from_u128(0x6444048F_4C8B_11D1_8B70_080036B11A03);

    let key_w = PROPERTYKEY {
        fmtid: psguid_imagesummaryinformation,
        pid: 3,
    };
    let val_w = unsafe { prop_store.GetValue(&key_w) }.expect("Get width property");
    assert_eq!(val_w.to_string(), "1024");

    let key_h = PROPERTYKEY {
        fmtid: psguid_imagesummaryinformation,
        pid: 4,
    };
    let val_h = unsafe { prop_store.GetValue(&key_h) }.expect("Get height property");
    assert_eq!(val_h.to_string(), "1024");

    let key_dim = PROPERTYKEY {
        fmtid: psguid_imagesummaryinformation,
        pid: 13,
    };
    let val_dim = unsafe { prop_store.GetValue(&key_dim) }.expect("Get dimensions property");
    assert_eq!(val_dim.to_string(), "1024 x 1024");
}
