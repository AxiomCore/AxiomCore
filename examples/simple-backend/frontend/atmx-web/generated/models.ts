// GENERATED CODE – DO NOT EDIT.
/* eslint-disable @typescript-eslint/no-explicit-any */


export interface PyExampleItem {
  description?: any;
  id: any;
  ownerId: any;
  title: any;
}


export interface PyExampleItemCreate {
  description?: any;
  title: any;
}


export interface PyExampleToken {
  accessToken: any;
  tokenType: any;
}


export interface PyExampleUser {
  email: any;
  id: any;
  role: any;
}


export interface PyExampleUserCreate {
  email: any;
  password: any;
}

export const Mappers: Record<string, any> = {
  pyExample: {
    Item: {
      fromJson: (json: any): PyExampleItem => ({
        description: json["description"],
        id: json["id"],
        ownerId: json["owner_id"],
        title: json["title"],
      }),
      toJson: (obj: any): any => ({
        "description": obj.description,
        "id": obj.id,
        "owner_id": obj.ownerId,
        "title": obj.title,
      })
    },
    ItemCreate: {
      fromJson: (json: any): PyExampleItemCreate => ({
        description: json["description"],
        title: json["title"],
      }),
      toJson: (obj: any): any => ({
        "description": obj.description,
        "title": obj.title,
      })
    },
    Token: {
      fromJson: (json: any): PyExampleToken => ({
        accessToken: json["access_token"],
        tokenType: json["token_type"],
      }),
      toJson: (obj: any): any => ({
        "access_token": obj.accessToken,
        "token_type": obj.tokenType,
      })
    },
    User: {
      fromJson: (json: any): PyExampleUser => ({
        email: json["email"],
        id: json["id"],
        role: json["role"],
      }),
      toJson: (obj: any): any => ({
        "email": obj.email,
        "id": obj.id,
        "role": obj.role,
      })
    },
    UserCreate: {
      fromJson: (json: any): PyExampleUserCreate => ({
        email: json["email"],
        password: json["password"],
      }),
      toJson: (obj: any): any => ({
        "email": obj.email,
        "password": obj.password,
      })
    },
  },
};
